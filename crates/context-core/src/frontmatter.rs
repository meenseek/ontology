use std::fmt::Display;

use serde::de::DeserializeOwned;
use serde_yaml_ng::{Mapping, Value};

use crate::{ContextError, Result};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ParsedFrontmatter {
    raw: String,
    mapping: Mapping,
}

impl ParsedFrontmatter {
    pub(crate) fn parse(raw: Option<&str>) -> Result<Option<Self>> {
        let Some(raw) = raw else {
            return Ok(None);
        };
        let value = serde_yaml_ng::from_str::<Value>(raw).map_err(|error| {
            ContextError::invalid_input(format!("invalid Markdown frontmatter YAML: {error}"))
        })?;
        let mapping = match value {
            Value::Mapping(mapping) => mapping,
            Value::Null => Mapping::new(),
            _ => {
                return Err(ContextError::invalid_input(
                    "Markdown frontmatter must be a YAML mapping",
                ));
            }
        };
        Ok(Some(Self {
            raw: raw.to_owned(),
            mapping,
        }))
    }

    pub(crate) fn from_markdown(markdown: &str) -> Result<Option<Self>> {
        let (raw, _) = split_markdown_frontmatter(markdown);
        if raw.is_none() && markdown.starts_with("---\n") {
            return Err(ContextError::invalid_input(
                "Markdown frontmatter is missing its closing delimiter",
            ));
        }
        Self::parse(raw)
    }

    pub(crate) fn raw(&self) -> &str {
        &self.raw
    }

    pub(crate) fn keys(&self) -> impl Iterator<Item = &str> {
        self.mapping.keys().filter_map(Value::as_str)
    }

    pub(crate) fn mapping_len(&self) -> usize {
        self.mapping.len()
    }

    pub(crate) fn optional_string(&self, field: &str) -> Result<Option<String>> {
        match self.value(field) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(value)) => Ok(Some(value.clone())),
            Some(_) => Err(field_type_error(field, "a string")),
        }
    }

    pub(crate) fn string_list(&self, field: &str) -> Result<Vec<String>> {
        let Some(value) = self.value(field) else {
            return Ok(Vec::new());
        };
        let Value::Sequence(values) = value else {
            return Err(field_type_error(field, "a sequence of strings"));
        };
        values
            .iter()
            .map(|value| match value {
                Value::String(value) => Ok(value.clone()),
                _ => Err(field_type_error(field, "a sequence of strings")),
            })
            .collect()
    }

    pub(crate) fn optional_bool(&self, field: &str) -> Result<Option<bool>> {
        match self.value(field) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Bool(value)) => Ok(Some(*value)),
            Some(_) => Err(field_type_error(field, "a boolean")),
        }
    }

    pub(crate) fn deserialize<T: DeserializeOwned>(&self) -> Result<T> {
        serde_yaml_ng::from_value(Value::Mapping(self.mapping.clone())).map_err(|error| {
            ContextError::invalid_input(format!("invalid Markdown frontmatter: {error}"))
        })
    }

    fn value(&self, field: &str) -> Option<&Value> {
        self.mapping.get(Value::String(field.to_owned()))
    }
}

pub(crate) fn split_markdown_frontmatter(content: &str) -> (Option<&str>, &str) {
    let Some(rest) = content.strip_prefix("---\n") else {
        return (None, content);
    };
    let Some(end) = rest.find("\n---\n") else {
        return (None, content);
    };
    let frontmatter = &rest[..end];
    let body = &rest[end + "\n---\n".len()..];
    (Some(frontmatter), body)
}

fn field_type_error(field: &str, expected: impl Display) -> ContextError {
    ContextError::invalid_input(format!(
        "Markdown frontmatter field `{field}` must be {expected}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_yaml_scalar_and_sequence_semantics() {
        let parsed = ParsedFrontmatter::parse(Some(
            "title: 'Don''t Split YAML'\naliases: [\"alpha,beta\", gamma]",
        ))
        .expect("valid frontmatter should parse")
        .expect("frontmatter should be present");

        assert_eq!(
            parsed
                .optional_string("title")
                .expect("title should be valid"),
            Some("Don't Split YAML".to_owned())
        );
        assert_eq!(
            parsed.string_list("aliases").expect("aliases should parse"),
            ["alpha,beta", "gamma"]
        );
    }

    #[test]
    fn rejects_non_mapping_frontmatter() {
        let error = ParsedFrontmatter::parse(Some("[one, two]"))
            .expect_err("a top-level sequence must be rejected");

        assert!(error.to_string().contains("must be a YAML mapping"));
    }

    #[test]
    fn resolves_alias_keys_in_the_full_yaml_document() {
        let parsed =
            ParsedFrontmatter::parse(Some("key_name: &custom-key custom\n*custom-key: keep"))
                .expect("valid frontmatter should parse")
                .expect("frontmatter should be present");

        assert_eq!(parsed.keys().collect::<Vec<_>>(), ["key_name", "custom"]);
    }
}
