use std::{
    collections::BTreeSet,
    path::{Component, Path},
};

use serde::Deserialize;

use crate::{ContextVaultError, Result, frontmatter::ParsedFrontmatter};

const MAX_ONTOLOGY_VALUE_BYTES: usize = 4_096;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct OntologyMetadata {
    #[serde(default)]
    ontology: bool,
    #[serde(default, rename = "type")]
    document_type: Option<String>,
    #[serde(default)]
    domain: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    confidence: Option<String>,
    #[serde(default)]
    entities: Vec<String>,
    #[serde(default)]
    relations: Vec<OntologyRelation>,
    #[serde(default)]
    applies_to: Vec<String>,
    #[serde(default)]
    related: Vec<String>,
    #[serde(default)]
    related_from_links: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct OntologyRelation {
    from: String,
    #[serde(rename = "type")]
    relation_type: String,
    to: String,
}

impl OntologyMetadata {
    pub(crate) fn parse(frontmatter: Option<&ParsedFrontmatter>) -> Result<Self> {
        let Some(frontmatter) = frontmatter else {
            return Ok(Self::default());
        };
        if frontmatter.optional_bool("ontology")? != Some(true) {
            return Ok(Self::default());
        }
        let metadata: Self = frontmatter.deserialize()?;
        metadata.validate()?;
        Ok(metadata)
    }

    #[must_use]
    pub fn document_type(&self) -> Option<&str> {
        self.document_type.as_deref()
    }

    #[must_use]
    pub fn domain(&self) -> Option<&str> {
        self.domain.as_deref()
    }

    #[must_use]
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    #[must_use]
    pub fn confidence(&self) -> Option<&str> {
        self.confidence.as_deref()
    }

    #[must_use]
    pub fn entities(&self) -> &[String] {
        &self.entities
    }

    #[must_use]
    pub fn relations(&self) -> &[OntologyRelation] {
        &self.relations
    }

    #[must_use]
    pub fn applies_to(&self) -> &[String] {
        &self.applies_to
    }

    #[must_use]
    pub fn related(&self) -> &[String] {
        &self.related
    }

    #[must_use]
    pub fn related_from_links(&self) -> bool {
        self.related_from_links
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.ontology
    }

    pub(crate) fn search_text(&self) -> String {
        let mut values = Vec::new();
        values.extend(self.document_type.iter().map(String::as_str));
        values.extend(self.domain.iter().map(String::as_str));
        values.extend(self.status.iter().map(String::as_str));
        values.extend(self.confidence.iter().map(String::as_str));
        values.extend(self.entities.iter().map(String::as_str));
        for relation in &self.relations {
            values.extend([
                relation.from.as_str(),
                relation.relation_type.as_str(),
                relation.to.as_str(),
            ]);
        }
        values.extend(self.applies_to.iter().map(String::as_str));
        values.extend(self.related.iter().map(String::as_str));
        values.join("\n")
    }

    fn validate(&self) -> Result<()> {
        for (field, value) in [
            ("type", self.document_type.as_deref()),
            ("domain", self.domain.as_deref()),
            ("status", self.status.as_deref()),
            ("confidence", self.confidence.as_deref()),
        ] {
            if let Some(value) = value {
                validate_value(field, value)?;
            }
        }
        validate_optional_vocabulary(
            "type",
            self.document_type.as_deref(),
            &[
                "project",
                "business",
                "decision",
                "knowledge",
                "writing",
                "learning",
                "entity",
            ],
        )?;
        validate_optional_vocabulary(
            "domain",
            self.domain.as_deref(),
            &[
                "portfolio",
                "product",
                "business",
                "engineering",
                "writing",
                "learning",
            ],
        )?;
        validate_optional_vocabulary(
            "status",
            self.status.as_deref(),
            &["draft", "active", "archived"],
        )?;
        validate_optional_vocabulary(
            "confidence",
            self.confidence.as_deref(),
            &["low", "medium", "high"],
        )?;
        for value in &self.entities {
            validate_entity_id("entities", value)?;
        }
        let mut relation_keys = BTreeSet::new();
        for relation in &self.relations {
            validate_entity_id("relations.from", &relation.from)?;
            validate_identifier("relations.type", &relation.relation_type)?;
            validate_entity_id("relations.to", &relation.to)?;
            if !relation_keys.insert((
                relation.from.as_str(),
                relation.relation_type.as_str(),
                relation.to.as_str(),
            )) {
                return Err(ContextVaultError::invalid_input(
                    "ontology relations must not contain duplicate edges",
                ));
            }
        }
        for value in &self.applies_to {
            validate_value("applies_to", value)?;
        }
        for value in &self.related {
            validate_related_path(value)?;
        }
        Ok(())
    }
}

fn validate_optional_vocabulary(field: &str, value: Option<&str>, allowed: &[&str]) -> Result<()> {
    if value.is_some_and(|value| !allowed.contains(&value)) {
        return Err(ContextVaultError::invalid_input(format!(
            "ontology field `{field}` is not in the supported vocabulary"
        )));
    }
    Ok(())
}

fn validate_identifier(field: &str, value: &str) -> Result<()> {
    validate_value(field, value)?;
    if value.starts_with('-')
        || value.ends_with('-')
        || !value.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
    {
        return Err(ContextVaultError::invalid_input(format!(
            "ontology field `{field}` must be a lowercase kebab-case identifier"
        )));
    }
    Ok(())
}

fn validate_entity_id(field: &str, value: &str) -> Result<()> {
    validate_value(field, value)?;
    let Some((kind, name)) = value.split_once(':') else {
        return Err(ContextVaultError::invalid_input(format!(
            "ontology field `{field}` must use `kind:name` entity IDs"
        )));
    };
    validate_identifier(field, kind)?;
    validate_identifier(field, name)
}

fn validate_related_path(value: &str) -> Result<()> {
    validate_value("related", value)?;
    let path = Path::new(value);
    if path.is_absolute()
        || path.extension().and_then(|extension| extension.to_str()) != Some("md")
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(ContextVaultError::invalid_input(
            "ontology field `related` must be a vault-relative Markdown path",
        ));
    }
    Ok(())
}

impl OntologyRelation {
    #[must_use]
    pub fn from(&self) -> &str {
        &self.from
    }

    #[must_use]
    pub fn relation_type(&self) -> &str {
        &self.relation_type
    }

    #[must_use]
    pub fn to(&self) -> &str {
        &self.to
    }
}

fn validate_value(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty()
        || value.contains(['\n', '\r'])
        || value.len() > MAX_ONTOLOGY_VALUE_BYTES
    {
        return Err(ContextVaultError::invalid_input(format!(
            "ontology field `{field}` must be a non-empty single-line value no larger than {MAX_ONTOLOGY_VALUE_BYTES} bytes"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> Result<OntologyMetadata> {
        let frontmatter = ParsedFrontmatter::parse(Some(raw))?;
        OntologyMetadata::parse(frontmatter.as_ref())
    }

    #[test]
    fn parses_ontology_projection_fields() {
        let metadata = parse(
            "ontology: true\ntype: project\ndomain: engineering\nstatus: active\nconfidence: high\nentities:\n  - project:llm-context-vault\nrelations:\n  - from: project:llm-context-vault\n    type: supports\n    to: knowledge:context-retrieval\napplies_to:\n  - writing\nrelated:\n  - personal/projects/llm-context-vault.md",
        )
        .expect("ontology metadata should parse");

        assert_eq!(metadata.document_type(), Some("project"));
        assert_eq!(metadata.domain(), Some("engineering"));
        assert_eq!(metadata.entities(), ["project:llm-context-vault"]);
        assert_eq!(metadata.relations()[0].relation_type(), "supports");
        assert!(
            metadata
                .search_text()
                .contains("knowledge:context-retrieval")
        );
    }

    #[test]
    fn link_relations_require_an_explicit_opt_in() {
        assert!(!parse("ontology: true").unwrap().related_from_links());
        assert!(
            parse("ontology: true\nrelated_from_links: true")
                .unwrap()
                .related_from_links()
        );
    }

    #[test]
    fn rejects_malformed_ontology_frontmatter() {
        let error = parse("entities: [unterminated").expect_err("malformed YAML must be rejected");

        assert!(
            error
                .to_string()
                .contains("invalid Markdown frontmatter YAML")
        );
    }

    #[test]
    fn ignores_generic_frontmatter_without_ontology_marker() {
        let metadata = parse("type: manual\nstatus: 3\nrelations:\n  owner: profile")
            .expect("generic frontmatter must not be treated as ontology");

        assert!(metadata.is_empty());
        assert!(metadata.search_text().is_empty());
    }

    #[test]
    fn rejects_duplicate_relation_edges() {
        let error = parse(
            "ontology: true\ntype: knowledge\ndomain: engineering\nrelations:\n  - from: knowledge:one\n    type: supports\n    to: project:two\n  - from: knowledge:one\n    type: supports\n    to: project:two",
        )
        .expect_err("duplicate ontology edges must be rejected");

        assert!(error.to_string().contains("duplicate edges"));
    }
}
