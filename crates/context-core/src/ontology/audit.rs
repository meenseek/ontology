//! Read-only checks against explicitly supplied definitions and selected sources.
//! Missing declarations describe the selection, never the existence or truth of an entity.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{OntologyMetadata, validate_identifier, validate_value};
use crate::{ContextError, Result};

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RelationDefinition {
    pub id: String,
    pub definition: String,
    pub from_kinds: Vec<String>,
    pub to_kinds: Vec<String>,
    pub allow_self: bool,
}

impl RelationDefinition {
    pub fn validate_all(definitions: &[Self]) -> Result<()> {
        if definitions.len() > 32 {
            return Err(ContextError::invalid_input(
                "at most 32 relation definitions are allowed",
            ));
        }
        let mut ids = BTreeSet::new();
        for definition in definitions {
            validate_identifier("definition.id", &definition.id)?;
            validate_value("definition.definition", &definition.definition)?;
            if !ids.insert(&definition.id) {
                return Err(ContextError::invalid_input(
                    "relation definition IDs must be unique",
                ));
            }
            for kinds in [&definition.from_kinds, &definition.to_kinds] {
                if kinds.is_empty() || kinds.len() > 32 {
                    return Err(ContextError::invalid_input(
                        "allowed entity kinds must contain 1 to 32 values",
                    ));
                }
                let mut unique = BTreeSet::new();
                for kind in kinds {
                    validate_identifier("definition.entity_kind", kind)?;
                    if !unique.insert(kind) {
                        return Err(ContextError::invalid_input(
                            "allowed entity kinds must be unique",
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct AuditDocument<'a> {
    pub path: &'a str,
    pub ontology: &'a OntologyMetadata,
}

#[derive(Debug, Serialize, Eq, PartialEq, Ord, PartialOrd)]
pub struct AuditFinding {
    pub code: &'static str,
    pub path: String,
    pub entity: Option<String>,
    pub relation_type: Option<String>,
}

#[derive(Debug, Serialize, Eq, PartialEq)]
pub struct AuditRelation {
    pub from: String,
    #[serde(rename = "type")]
    pub relation_type: String,
    pub to: String,
    pub sources: Vec<String>,
}

#[derive(Debug, Serialize, Eq, PartialEq)]
pub struct AuditPredicateCount {
    #[serde(rename = "type")]
    pub relation_type: String,
    pub count: usize,
}

#[derive(Debug, Serialize, Eq, PartialEq)]
pub struct AuditReport {
    pub documents: usize,
    pub listed_entities: usize,
    pub assertions: usize,
    pub predicate_counts: Vec<AuditPredicateCount>,
    pub relations: Vec<AuditRelation>,
    pub findings: Vec<AuditFinding>,
}

pub fn audit(
    documents: &[AuditDocument<'_>],
    definitions: Option<&[RelationDefinition]>,
) -> Result<AuditReport> {
    if let Some(definitions) = definitions {
        RelationDefinition::validate_all(definitions)?;
    }
    let criteria_supplied = definitions.is_some();
    let mut paths = BTreeSet::new();
    let mut entities = BTreeSet::new();
    let mut findings = BTreeSet::new();
    for document in documents {
        if !paths.insert(document.path) {
            return Err(ContextError::invalid_input(
                "audit source paths must be unique",
            ));
        }
        let mut local = BTreeSet::new();
        for entity in document.ontology.entities() {
            entities.insert(entity.as_str());
            if !local.insert(entity) {
                findings.insert(finding(
                    "duplicate-entity-declaration",
                    document.path,
                    Some(entity),
                    None,
                ));
            }
        }
    }
    let definitions: BTreeMap<_, _> = definitions
        .unwrap_or_default()
        .iter()
        .map(|definition| (definition.id.as_str(), definition))
        .collect();
    let mut assertions = 0;
    let mut predicate_counts = BTreeMap::new();
    let mut edges = BTreeMap::<(&str, &str, &str), BTreeSet<&str>>::new();
    for document in documents {
        for relation in document.ontology.relations() {
            assertions += 1;
            *predicate_counts
                .entry(relation.relation_type().to_owned())
                .or_insert(0) += 1;
            edges
                .entry((relation.from(), relation.relation_type(), relation.to()))
                .or_default()
                .insert(document.path);
            for endpoint in [relation.from(), relation.to()] {
                if !entities.contains(endpoint) {
                    findings.insert(finding(
                        "endpoint-not-listed-in-selected-entities",
                        document.path,
                        Some(endpoint),
                        Some(relation.relation_type()),
                    ));
                }
            }
            if !criteria_supplied {
                continue;
            }
            let Some(definition) = definitions.get(relation.relation_type()) else {
                findings.insert(finding(
                    "relation-type-not-defined-in-input",
                    document.path,
                    None,
                    Some(relation.relation_type()),
                ));
                continue;
            };
            for (endpoint, allowed, code) in [
                (
                    relation.from(),
                    &definition.from_kinds,
                    "source-kind-not-allowed",
                ),
                (
                    relation.to(),
                    &definition.to_kinds,
                    "target-kind-not-allowed",
                ),
            ] {
                let kind = endpoint
                    .split_once(':')
                    .map(|(kind, _)| kind)
                    .unwrap_or_default();
                if !allowed.iter().any(|allowed| allowed == kind) {
                    findings.insert(finding(
                        code,
                        document.path,
                        Some(endpoint),
                        Some(relation.relation_type()),
                    ));
                }
            }
            if !definition.allow_self && relation.from() == relation.to() {
                findings.insert(finding(
                    "self-relation-not-allowed",
                    document.path,
                    Some(relation.from()),
                    Some(relation.relation_type()),
                ));
            }
        }
    }
    Ok(AuditReport {
        documents: documents.len(),
        listed_entities: entities.len(),
        assertions,
        predicate_counts: predicate_counts
            .into_iter()
            .map(|(relation_type, count)| AuditPredicateCount {
                relation_type,
                count,
            })
            .collect(),
        relations: edges
            .into_iter()
            .map(|((from, relation_type, to), sources)| AuditRelation {
                from: from.to_owned(),
                relation_type: relation_type.to_owned(),
                to: to.to_owned(),
                sources: sources.into_iter().map(str::to_owned).collect(),
            })
            .collect(),
        findings: findings.into_iter().collect(),
    })
}

fn finding(
    code: &'static str,
    path: &str,
    entity: Option<&str>,
    relation_type: Option<&str>,
) -> AuditFinding {
    AuditFinding {
        code,
        path: path.to_owned(),
        entity: entity.map(str::to_owned),
        relation_type: relation_type.map(str::to_owned),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontmatter::ParsedFrontmatter;

    fn metadata(entities: &str, from: &str, to: &str) -> OntologyMetadata {
        let raw = format!(
            "ontology: true\nentities: [{entities}]\nrelations:\n  - from: {from}\n    type: depends-on\n    to: {to}\n"
        );
        OntologyMetadata::parse(ParsedFrontmatter::parse(Some(&raw)).unwrap().as_ref()).unwrap()
    }
    fn definition() -> RelationDefinition {
        RelationDefinition {
            id: "depends-on".into(),
            definition: "The source requires the target to operate.".into(),
            from_kinds: vec!["project".into()],
            to_kinds: vec!["system".into()],
            allow_self: false,
        }
    }
    #[test]
    fn selected_declarations_and_contracts_are_distinct_from_assertions() {
        let source = metadata("project:one", "project:one", "system:two");
        let documents = [AuditDocument {
            path: "a.md",
            ontology: &source,
        }];
        assert_eq!(audit(&documents, None).unwrap().findings.len(), 1);
        let report = audit(&documents, Some(&[])).unwrap();
        assert_eq!(report.assertions, 1);
        assert_eq!(
            report.findings.iter().map(|f| f.code).collect::<Vec<_>>(),
            [
                "endpoint-not-listed-in-selected-entities",
                "relation-type-not-defined-in-input"
            ]
        );
        let report = audit(&documents, Some(&[definition()])).unwrap();
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].entity.as_deref(), Some("system:two"));
    }
    #[test]
    fn repeated_assertions_retain_all_sources_and_do_not_merge_entities() {
        let first = metadata("project:one, system:two", "project:one", "system:two");
        let second = metadata("project:other, system:two", "project:one", "system:two");
        let documents = [
            AuditDocument {
                path: "b.md",
                ontology: &second,
            },
            AuditDocument {
                path: "a.md",
                ontology: &first,
            },
        ];
        let report = audit(&documents, Some(&[definition()])).unwrap();
        assert!(report.findings.is_empty());
        assert_eq!(report.listed_entities, 3);
        assert_eq!(report.assertions, 2);
        assert_eq!(report.relations.len(), 1);
        assert_eq!(report.relations[0].sources, ["a.md", "b.md"]);
        assert_eq!(
            report,
            audit(&[documents[1], documents[0]], Some(&[definition()])).unwrap()
        );
    }
    #[test]
    fn direction_and_self_policy_are_checked_without_rewriting_edges() {
        let source = metadata("system:one, system:one", "system:one", "system:one");
        let report = audit(
            &[AuditDocument {
                path: "a.md",
                ontology: &source,
            }],
            Some(&[definition()]),
        )
        .unwrap();
        assert_eq!(
            report.findings.iter().map(|f| f.code).collect::<Vec<_>>(),
            [
                "duplicate-entity-declaration",
                "self-relation-not-allowed",
                "source-kind-not-allowed"
            ]
        );
        assert_eq!(report.relations[0].from, "system:one");
    }
    #[test]
    fn definitions_require_explicit_and_unique_endpoint_rules() {
        let mut value = definition();
        value.to_kinds.clear();
        assert!(RelationDefinition::validate_all(&[value]).is_err());
        assert!(RelationDefinition::validate_all(&[definition(), definition()]).is_err());
        let source = metadata("project:one, system:two", "project:one", "system:two");
        assert!(
            audit(
                &[
                    AuditDocument {
                        path: "a.md",
                        ontology: &source
                    },
                    AuditDocument {
                        path: "a.md",
                        ontology: &source
                    }
                ],
                None
            )
            .is_err()
        );
    }
}
