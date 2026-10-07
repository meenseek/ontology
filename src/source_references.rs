//! Authored link metadata. These paths are discovery references, never evidence.
use crate::{
    context::{ContextScope, restricted, validate_path},
    context_projection::eligible,
    domain::{Error, Scope, validate_id},
    store::{Store, digest},
};
use pulldown_cmark::{Event, Parser, Tag};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::Row;
use std::collections::{BTreeSet, HashSet};

fn destination(href: &str) -> Option<String> {
    let path = href.split('#').next()?;
    if path.is_empty() || path.contains('?') || path.split('/').next()?.contains(':') {
        return None;
    }
    let mut decoded = Vec::with_capacity(path.len());
    let mut bytes = path.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let hi = char::from(bytes.next()?).to_digit(16)?;
            let lo = char::from(bytes.next()?).to_digit(16)?;
            decoded.push((hi * 16 + lo) as u8);
        } else {
            decoded.push(byte);
        }
    }
    let path = String::from_utf8(decoded).ok()?;
    if path.starts_with('/') || path.contains(['\\', '\0']) || path.chars().any(char::is_control) {
        return None;
    }
    Some(path)
}
fn relative(path: &str, href: &str) -> Option<String> {
    let href = destination(href)?;
    let mut parts: Vec<&str> = path.split('/').collect();
    parts.pop();
    for part in href.split('/') {
        match part {
            "" => return None,
            "." => {}
            ".." => {
                parts.pop()?;
            }
            value => parts.push(value),
        }
    }
    let target = parts.join("/");
    if target == path || validate_path(&target).is_err() || !target.ends_with(".md") {
        return None;
    }
    Some(target)
}
// Git accepts arbitrary UTF-8 Markdown. Exclude only a closed initial metadata
// block, without validating YAML or consuming later authored separators.
fn git_body(content: &str) -> &str {
    let mut lines = content.split_inclusive('\n');
    let Some(first) = lines.next() else {
        return content;
    };
    if first
        .trim_start_matches('\u{feff}')
        .trim_end_matches(['\r', '\n'])
        != "---"
    {
        return content;
    }
    let mut offset = first.len();
    for line in lines {
        offset += line.len();
        if line.trim_end_matches(['\r', '\n']) == "---" {
            return &content[offset..];
        }
    }
    content
}
pub(crate) fn paths(path: &str, body: &str, native: bool) -> Vec<String> {
    Parser::new(if native { body } else { git_body(body) })
        .filter_map(|event| {
            let Event::Start(Tag::Link { dest_url, .. }) = event else {
                return None;
            };
            let target = relative(path, dest_url.as_ref())?;
            if native {
                let (first, rest) = target.split_once('/')?;
                let (scope, target_path) = if first == "work" {
                    let (company, path) = rest.split_once('/')?;
                    (format!("work/{company}"), path)
                } else {
                    (first.to_owned(), rest)
                };
                let scope: ContextScope = scope.parse().ok()?;
                if restricted(target_path) || !eligible(&scope, target_path, false) {
                    return None;
                }
            } else if target.split('/').any(|part| part == ".git") {
                return None;
            }
            Some(target)
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImportedReferenceSource {
    pub entity_id: String,
    pub source_revision: String,
    pub content_digest: String,
}
impl Store {
    /// Refresh only derived paths from current, explicitly reviewed Git originals.
    pub async fn refresh_imported_references(
        &self,
        scope: Scope,
        sources: &[ImportedReferenceSource],
    ) -> Result<Value, Error> {
        if sources.is_empty() || sources.len() > 100 {
            return Err(Error::Invalid);
        }
        let mut unique = HashSet::new();
        for source in sources {
            validate_id(&source.entity_id)?;
            if !unique.insert(&source.entity_id)
                || source.content_digest.len() != 64
                || !source
                    .content_digest
                    .bytes()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
                || !matches!(source.source_revision.len(), 40 | 64)
                || !source
                    .source_revision
                    .bytes()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            {
                return Err(Error::Invalid);
            }
        }
        self.count(1);
        let mut tx = self.pool().begin().await.map_err(|_| Error::Storage)?;
        let ids: Vec<&str> = sources.iter().map(|s| s.entity_id.as_str()).collect();
        self.count(1);
        let rows = sqlx::query("SELECT e.id,s.path,s.status,s.verified_revision,r.content,r.source_revision,r.content_digest,r.present FROM entities e JOIN sources s ON s.scope=e.scope AND s.id=e.source_id JOIN source_records r ON r.scope=e.scope AND r.entity_id=e.id WHERE e.scope=$1 AND e.id=ANY($2) AND s.kind='git' AND NOT EXISTS(SELECT 1 FROM context_source_bindings b WHERE b.source_id=s.id) ORDER BY e.id FOR UPDATE OF s,r")
            .bind(scope.as_str()).bind(ids).fetch_all(&mut *tx).await.map_err(|_| Error::Storage)?;
        if rows.len() != sources.len() {
            return Err(Error::Conflict);
        }
        let mut projections = Vec::with_capacity(rows.len());
        for row in rows {
            let id: String = row.get("id");
            let expected = sources
                .iter()
                .find(|s| s.entity_id == id)
                .ok_or(Error::Storage)?;
            let content: Option<String> = row.get("content");
            let content = content.ok_or(Error::Conflict)?;
            if row.get::<String, _>("status") != "ok"
                || !row.get::<bool, _>("present")
                || row.get::<Option<String>, _>("source_revision").as_deref()
                    != Some(&expected.source_revision)
                || row.get::<Option<String>, _>("verified_revision").as_deref()
                    != Some(&expected.source_revision)
                || row.get::<Option<String>, _>("content_digest").as_deref()
                    != Some(&expected.content_digest)
                || digest(content.as_bytes()) != expected.content_digest
            {
                return Err(Error::Conflict);
            }
            projections.push(json!({"entity_id":id,"reference_paths":paths(&row.get::<String,_>("path"),&content,false)}));
        }
        self.count(1);
        let changed = sqlx::query("UPDATE source_records r SET reference_paths=i.reference_paths FROM jsonb_to_recordset($1) AS i(entity_id text,reference_paths text[]) WHERE r.scope=$2 AND r.entity_id=i.entity_id AND r.reference_paths IS DISTINCT FROM i.reference_paths")
            .bind(json!(projections)).bind(scope.as_str()).execute(&mut *tx).await.map_err(|_| Error::Storage)?.rows_affected();
        self.count(1);
        tx.commit().await.map_err(|_| Error::Storage)?;
        Ok(json!({"reviewed":sources.len(),"changed":changed}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authored_links_resolve_exact_paths_without_images_code_or_metadata() {
        let body = "---\nfake: '[metadata](metadata.md)'\n---\n[one](../%ED%95%9C%EA%B8%80%20name.md#section) [again](<../한글 name.md>) [dot](./b.md) [self](a.md#x)\n![image](image.md) `[code](code.md)`\n```md\n[fence](fence.md)\n```\n[query](b.md?q=1) [web](https://example.com/a.md) [absolute](/a.md) [escape](../../escape.md) [bad](bad%GG.md) [encoded escape](%2E%2E/%2E%2E/x.md)";
        assert_eq!(
            paths("notes/a.md", body, false),
            vec!["notes/b.md", "한글 name.md"]
        );
    }
    #[test]
    fn native_links_cross_only_valid_scopes_and_exclude_restricted_originals() {
        let body = "[own](b.md) [profile](../../profile/rules/one.md#x) [work](../../work/common/index.md) [raw](../raw/private.md) [journal](../journal/day.md) [bad scope](../../other/a.md) [escape](../../../personal/a.md)";
        assert_eq!(
            paths("personal/notes/a.md", body, true),
            vec![
                "personal/notes/b.md",
                "profile/rules/one.md",
                "work/common/index.md"
            ]
        );
    }
    #[test]
    fn encoded_filename_punctuation_is_not_a_scheme_query_or_fragment() {
        assert_eq!(
            paths(
                "personal/a.md",
                "[colon](part%3Aname.md) [question](name%3Fpart.md) [hash](name%23part.md)",
                true
            ),
            vec!["personal/name#part.md", "personal/part:name.md"]
        );
    }
    #[test]
    fn encoded_git_filename_question_mark_is_literal() {
        assert_eq!(
            paths(
                "a.md",
                "[question](name%3Fpart.md) [query](name.md?part)",
                false
            ),
            vec!["name?part.md"]
        );
    }
    #[test]
    fn native_projection_does_not_parse_authored_body_as_metadata_again() {
        let bytes = b"---\ntitle: Source\n---\n---\n[real](b.md)\n---\n";
        let scope: ContextScope = "personal".parse().unwrap();
        let projected = crate::context_projection::projection(
            &scope,
            "a.md",
            &digest(bytes),
            false,
            false,
            Some(bytes),
        )
        .unwrap();
        assert_eq!(projected["source_references"], json!(["personal/b.md"]));
    }
    #[test]
    fn git_only_excludes_closed_initial_metadata_and_preserves_body_sections() {
        let body = "---\ninvalid: '[metadata](meta.md)'\n---\n# Source\n\n---\n[real](b.md)\n---\n";
        assert_eq!(paths("a.md", body, false), vec!["b.md"]);
        assert_eq!(
            paths("a.md", "# Source\n\n---\n[real](b.md)\n---\n", false),
            vec!["b.md"]
        );
        assert_eq!(paths("a.md", "---\n[real](b.md)\n", false), vec!["b.md"]);
        assert_eq!(
            paths(
                "a.md",
                "\u{feff}---\r\ninvalid: '[metadata](meta.md)'\r\n---\r\n[real](b.md)\r\n",
                false
            ),
            vec!["b.md"]
        );
    }
}
