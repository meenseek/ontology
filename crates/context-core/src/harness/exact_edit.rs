//! Explicit literal edits: candidate construction only, never semantic approval.
use super::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExactReplacement {
    pub before: String,
    pub after: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExactDocumentEdit {
    pub path: String,
    pub expected_content_digest: String,
    pub replacements: Vec<ExactReplacement>,
}

pub(super) fn validate_exact_edit_targets(
    edits: &[ExactDocumentEdit],
    targets: &[String],
) -> HarnessResult<()> {
    if edits.is_empty() || edits.len() != targets.len() || edits.len() > MAX_TARGETS {
        return Err(HarnessError::InvalidRequest(
            "exact edits must cover every planned target once".to_owned(),
        ));
    }
    let mut paths = BTreeSet::new();
    let mut effective = false;
    for edit in edits {
        validate_digest("exact edit source digest", &edit.expected_content_digest)?;
        if !targets.contains(&edit.path)
            || !paths.insert(&edit.path)
            || edit.replacements.len() > MAX_SUBMISSION_LIST_ITEMS
        {
            return Err(HarnessError::InvalidRequest(
                "exact edit targets are duplicated, outside scope or oversized".to_owned(),
            ));
        }
        for replacement in &edit.replacements {
            if replacement.before.is_empty()
                || replacement.before == replacement.after
                || replacement.before.len() > MAX_CANDIDATE_BYTES
                || replacement.after.len() > MAX_CANDIDATE_BYTES
            {
                return Err(HarnessError::InvalidRequest(
                    "exact edits require nonempty, effective, bounded literal matches".to_owned(),
                ));
            }
            effective = true;
        }
    }
    if !effective {
        return Err(HarnessError::InvalidRequest(
            "exact edit request has no changes".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn validate_exact_edit_bindings(
    edits: &[ExactDocumentEdit],
    targets: &[TargetBinding],
) -> HarnessResult<()> {
    validate_exact_edit_targets(
        edits,
        &targets
            .iter()
            .map(|target| target.workspace_relative_path.clone())
            .collect::<Vec<_>>(),
    )?;
    for edit in edits {
        if !targets.iter().any(|target| target.workspace_relative_path == edit.path
            && matches!(&target.state, TargetState::Existing { content_digest } if content_digest == &edit.expected_content_digest)) {
            return Err(HarnessError::InvalidRequest("exact edits require existing targets with their current digest".to_owned()));
        }
    }
    Ok(())
}

/// Compute the full Writer candidate from exact, bound original UTF-8 bytes.
/// All ranges refer to the original simultaneously; replacements never cascade.
pub fn exact_edit_changes<'a>(
    edits: &[ExactDocumentEdit],
    documents: impl IntoIterator<Item = &'a HarnessBoundDocument>,
) -> HarnessResult<Vec<FileChange>> {
    let documents = documents
        .into_iter()
        .filter(|document| document.source == HarnessBoundDocumentSource::Target)
        .collect::<Vec<_>>();
    validate_exact_edit_targets(
        edits,
        &documents
            .iter()
            .map(|document| document.relative_path.clone())
            .collect::<Vec<_>>(),
    )?;
    let mut changes = Vec::with_capacity(edits.len());
    let mut total_bytes = 0_usize;
    for edit in edits {
        let document = documents
            .iter()
            .find(|document| document.relative_path == edit.path)
            .ok_or_else(|| {
                HarnessError::InvalidSubmission("exact edit has no bound target".to_owned())
            })?;
        if documents
            .iter()
            .filter(|document| document.relative_path == edit.path)
            .count()
            != 1
            || document.content_digest != edit.expected_content_digest
            || byte_digest(document.content.as_bytes()) != document.content_digest
        {
            return Err(HarnessError::InvalidSubmission(
                "exact edit source digest differs from the bound bytes".to_owned(),
            ));
        }
        let original = &document.content;
        let mut ranges = Vec::with_capacity(edit.replacements.len());
        for replacement in &edit.replacements {
            let start = original.find(&replacement.before).ok_or_else(|| {
                HarnessError::InvalidSubmission("exact edit does not match its target".to_owned())
            })?;
            if original.rfind(&replacement.before) != Some(start) {
                return Err(HarnessError::InvalidSubmission(
                    "exact edit match is ambiguous".to_owned(),
                ));
            }
            ranges.push((start, start + replacement.before.len(), &replacement.after));
        }
        ranges.sort_unstable_by_key(|range| range.0);
        let mut content = String::new();
        let mut cursor = 0;
        for (start, end, after) in ranges {
            if start < cursor {
                return Err(HarnessError::InvalidSubmission(
                    "exact edit ranges overlap".to_owned(),
                ));
            }
            if content
                .len()
                .saturating_add(start - cursor)
                .saturating_add(after.len())
                > MAX_CANDIDATE_BYTES
            {
                return Err(HarnessError::InvalidSubmission(
                    "exact edit candidate exceeds the byte limit".to_owned(),
                ));
            }
            content.push_str(&original[cursor..start]);
            content.push_str(after);
            cursor = end;
        }
        if content.len().saturating_add(original.len() - cursor) > MAX_CANDIDATE_BYTES {
            return Err(HarnessError::InvalidSubmission(
                "exact edit candidate exceeds the byte limit".to_owned(),
            ));
        }
        content.push_str(&original[cursor..]);
        execution::validate_artifact_text("exact edit candidate", &content)?;
        total_bytes = total_bytes.saturating_add(content.len());
        if total_bytes > MAX_CANDIDATE_BYTES {
            return Err(HarnessError::InvalidSubmission(
                "exact edit candidates exceed the total byte limit".to_owned(),
            ));
        }
        changes.push(FileChange::Update {
            path: edit.path.clone(),
            expected_content_digest: edit.expected_content_digest.clone(),
            content,
        });
    }
    Ok(changes)
}
