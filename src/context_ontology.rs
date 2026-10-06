//! Exact-path diagnostic reads. No projection, original, relationship or policy is changed.
use crate::{
    context::ContextScope,
    context_projection::normalize_paths,
    domain::{Error, MAX_DOCUMENT_BYTES, MAX_RESPONSE_BYTES},
    store::{Store, digest},
};
use context_core::{
    document::parse_markdown_bytes,
    ontology::{AuditDocument, RelationDefinition, audit},
    redaction::redact_secrets,
};
use serde_json::{Value, json};
use sqlx::Row;
use std::path::Path;

impl Store {
    pub async fn audit_context_ontology(
        &self,
        scope: &ContextScope,
        paths: &[String],
        definitions: Option<&[RelationDefinition]>,
    ) -> Result<Value, Error> {
        let mut paths = normalize_paths(scope, paths)?;
        paths.sort();
        if paths.is_empty() || paths.iter().any(|path| !path.ends_with(".md")) {
            return Err(Error::Invalid);
        }
        if let Some(definitions) = definitions {
            RelationDefinition::validate_all(definitions).map_err(|_| Error::Invalid)?;
        }
        let mut tx = self.lock_context(false).await?;
        let result = async {
            self.count(1);
            // All source bytes, provenance and transfer bounds use the same SQL snapshot.
            let rows = sqlx::query(r#"
                WITH selected AS MATERIALIZED (
                    SELECT path,material_id,revision,content_digest,byte_len,deleted,restricted,content
                    FROM context_materials WHERE scope=$1 AND path=ANY($2)
                ), bounds AS (
                    SELECT count(*) AS found,
                        COALESCE(bool_and(NOT deleted AND NOT restricted),false) AS eligible,
                        COALESCE(bool_and(byte_len BETWEEN 0 AND $4
                            AND octet_length(content) BETWEEN 0 AND $4
                            AND byte_len=octet_length(content)),false) AS bounded,
                        COALESCE(sum(byte_len),0)::bigint AS stored_total,
                        COALESCE(sum(octet_length(content)),0)::bigint AS actual_total
                    FROM selected
                )
                SELECT s.path,s.material_id::text,s.revision,s.content_digest,s.byte_len,
                    s.deleted,s.restricted,octet_length(s.content) AS actual_bytes,
                    b.stored_total,b.actual_total,c.store_id::text,
                    CASE WHEN b.found=$3 AND b.eligible AND b.bounded
                        AND b.stored_total<=$5 AND b.actual_total<=$5 THEN s.content END AS content
                FROM selected s CROSS JOIN bounds b CROSS JOIN context_store c
                ORDER BY s.path COLLATE "C"
            "#)
            .bind(scope.as_str()).bind(&paths).bind(paths.len() as i64)
            .bind(MAX_DOCUMENT_BYTES as i64).bind(MAX_RESPONSE_BYTES as i64)
            .fetch_all(&mut *tx).await.map_err(|_| Error::Storage)?;
            if rows.len() != paths.len() || rows.iter().any(|row| row.get::<bool,_>("deleted") || row.get::<bool,_>("restricted")) {
                return Err(Error::NotFound);
            }
            let mut parsed = Vec::with_capacity(rows.len());
            let mut sources = Vec::with_capacity(rows.len());
            for row in &rows {
                let byte_len: i64 = row.get("byte_len");
                let actual_bytes: i32 = row.get("actual_bytes");
                if byte_len < 0 || byte_len != i64::from(actual_bytes) { return Err(Error::Storage); }
                if byte_len > MAX_DOCUMENT_BYTES as i64 || row.get::<i64,_>("stored_total") > MAX_RESPONSE_BYTES as i64 || row.get::<i64,_>("actual_total") > MAX_RESPONSE_BYTES as i64 {
                    return Err(Error::Limit);
                }
                let bytes: Vec<u8> = row.get::<Option<Vec<u8>>,_>("content").ok_or(Error::Storage)?;
                let sha: String = row.get("content_digest");
                if digest(&bytes) != sha || bytes.len() as i64 != byte_len { return Err(Error::Storage); }
                let path: String = row.get("path");
                let logical_path = format!("{}/{path}", scope.as_str());
                let source = parse_markdown_bytes(Path::new(&logical_path), &bytes).map_err(|_| Error::Invalid)?;
                if source.declared_scope().is_some_and(|declared| declared != scope.as_str().split('/').next().unwrap_or_default()) {
                    return Err(Error::Invalid);
                }
                sources.push(json!({"store_id":row.get::<String,_>("store_id"),"scope":scope,"path":path,"material_id":row.get::<String,_>("material_id"),"revision":row.get::<i64,_>("revision"),"content_digest":sha,"byte_len":byte_len}));
                parsed.push(source);
            }
            let documents: Vec<_> = parsed.iter().zip(&paths).map(|(source, path)| AuditDocument { path, ontology: source.document().ontology() }).collect();
            let report = audit(&documents, definitions).map_err(|_| Error::Invalid)?;
            let criteria = json!({"supplied":definitions.is_some(),"definitions":definitions});
            let criteria_digest = digest(&serde_json::to_vec(&criteria).map_err(|_| Error::Storage)?);
            let selection_digest = digest(&serde_json::to_vec(&sources).map_err(|_| Error::Storage)?);
            let mut report = serde_json::to_value(report).map_err(|_| Error::Storage)?;
            let mut criteria = criteria;
            let redacted = redact_strings(&mut report) | redact_strings(&mut criteria);
            let value = json!({"purpose":"diagnostic","selection_only":true,"truth_assessed":false,"redacted":redacted,"scope":scope,"selection_digest":selection_digest,"criteria_digest":criteria_digest,"sources":sources,"criteria":criteria,"report":report});
            if serde_json::to_vec(&value).map_err(|_| Error::Storage)?.len() > MAX_RESPONSE_BYTES { return Err(Error::Limit); }
            Ok(value)
        }.await;
        self.finish_context(tx, result).await
    }
}

fn redact_strings(value: &mut Value) -> bool {
    match value {
        Value::String(text) => {
            let redacted = redact_secrets(text);
            let changed = redacted != *text;
            *text = redacted;
            changed
        }
        Value::Array(values) => values
            .iter_mut()
            .fold(false, |changed, value| redact_strings(value) | changed),
        Value::Object(values) => values
            .iter_mut()
            // Source references are caller-selected identifiers and must still join to provenance.
            .filter(|(key, _)| !matches!(key.as_str(), "path" | "sources"))
            .fold(false, |changed, (_, value)| redact_strings(value) | changed),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn predicate_counts_and_criteria_cannot_expose_redacted_identifiers() {
        let bytes = b"---\nontology: true\nentities: [project:one, system:two]\nrelations:\n  - from: project:one\n    type: sk-synthetic123456789\n    to: system:two\n---\nSynthetic source.\n";
        let source = parse_markdown_bytes(Path::new("personal/source.md"), bytes).unwrap();
        let definition = RelationDefinition {
            id: "sk-synthetic123456789".into(),
            definition: "A synthetic relation.".into(),
            from_kinds: vec!["project".into()],
            to_kinds: vec!["system".into()],
            allow_self: false,
        };
        let report = audit(
            &[AuditDocument {
                path: "source.md",
                ontology: source.document().ontology(),
            }],
            Some(std::slice::from_ref(&definition)),
        )
        .unwrap();
        let mut value = json!({"report":report,"criteria":{"definitions":[definition]}});
        assert!(redact_strings(&mut value));
        assert_eq!(value["report"]["predicate_counts"][0]["type"], "[REDACTED]");
        assert_eq!(value["report"]["predicate_counts"][0]["count"], 1);
        assert!(!value.to_string().contains("sk-synthetic123456789"));
        assert_eq!(source.original_content_digest(), digest(bytes));
    }

    #[test]
    fn redaction_preserves_exact_source_references_for_every_assertion() {
        let bytes = b"---\nontology: true\nrelations:\n  - from: project:sk-synthetic123456789\n    type: depends-on\n    to: system:two\n---\nSynthetic source.\n";
        let source = parse_markdown_bytes(Path::new("personal/source.md"), bytes).unwrap();
        let first = "a-sk-synthetic123456789.md";
        let second = "a-sk-synthetic987654321.md";
        let report = audit(
            &[
                AuditDocument {
                    path: first,
                    ontology: source.document().ontology(),
                },
                AuditDocument {
                    path: second,
                    ontology: source.document().ontology(),
                },
            ],
            None,
        )
        .unwrap();
        let mut report = serde_json::to_value(report).unwrap();
        assert!(redact_strings(&mut report));
        assert_eq!(report["relations"][0]["sources"], json!([first, second]));
        let finding_paths: std::collections::BTreeSet<_> = report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|finding| finding["path"].as_str().unwrap())
            .collect();
        assert_eq!(finding_paths, [first, second].into_iter().collect());
        assert_eq!(report["relations"][0]["from"], "project:[REDACTED]");
    }
}
