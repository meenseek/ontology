use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

pub const MAX_DOCUMENT_BYTES: usize = 65_536;
pub const MAX_RESULTS: usize = 100;
pub const MAX_RESPONSE_BYTES: usize = 1_048_576;
pub const MAX_SEARCH_CHARS: usize = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Meenseek,
    Personal,
}
impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Meenseek => "meenseek",
            Self::Personal => "personal",
        }
    }
}
impl FromStr for Scope {
    type Err = Error;
    fn from_str(value: &str) -> Result<Self, Error> {
        match value {
            "meenseek" => Ok(Self::Meenseek),
            "personal" => Ok(Self::Personal),
            _ => Err(Error::Invalid),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Git,
    Vault,
}
impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Git => "git",
            Self::Vault => "vault",
        }
    }
    pub fn failure_code(self) -> &'static str {
        match self {
            Self::Git => "git-read-failed",
            Self::Vault => "vault-read-failed",
        }
    }
}

// The fixed company taxonomy is owned here; the UI consumes it through the API.
pub const AREAS: [(&str, &str); 5] = [
    ("strategy-portfolio", "전략·포트폴리오"),
    ("market-customer", "시장·고객 이해"),
    ("product-service", "제품·서비스 제공"),
    ("growth-customer", "성장·판매·고객 관계"),
    ("business-operations", "경영 기반"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Invalid,
    NotFound,
    Conflict,
    Forbidden,
    Storage,
    Baseline,
    Import,
    Limit,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "Invalid request",
            Self::NotFound => "Record not found",
            Self::Conflict => "Revision conflict; refresh the record",
            Self::Forbidden => "Local session verification failed",
            Self::Storage => "Database operation failed",
            Self::Baseline => "Baseline mismatch or nonempty uninitialized database",
            Self::Import => "Source verification failed",
            Self::Limit => "Operation limit exceeded",
        })
    }
}
impl std::error::Error for Error {}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Classification {
    pub revision: i64,
    pub areas: Vec<String>,
    pub topics: Vec<String>,
}
impl Classification {
    pub fn validate(&self, scope: Scope) -> Result<(), Error> {
        if self.revision < 0
            || self.revision == i64::MAX
            || self.areas.len() > 5
            || self.topics.len() > 10
        {
            return Err(Error::Invalid);
        }
        if self
            .areas
            .iter()
            .any(|area| scope != Scope::Meenseek || !AREAS.iter().any(|(id, _)| area == id))
        {
            return Err(Error::Invalid);
        }
        if self.topics.iter().any(|topic| {
            topic.trim() != topic
                || topic.is_empty()
                || topic.chars().count() > 80
                || topic.chars().any(char::is_control)
        }) {
            return Err(Error::Invalid);
        }
        if self
            .areas
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != self.areas.len()
            || self
                .topics
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                != self.topics.len()
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkChange {
    pub revision: i64,
    pub target_id: String,
    pub remove: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct ImportedRecord {
    pub source_id: String,
    pub entity_id: String,
    pub scope: Scope,
    pub repository: String,
    pub path: String,
    pub kind: SourceKind,
    pub source_revision: String,
    pub digest: Option<String>,
    pub content: Option<String>,
}

pub fn validate_search(query: &str) -> Result<(), Error> {
    if query.chars().count() > MAX_SEARCH_CHARS || query.contains('\0') {
        Err(Error::Invalid)
    } else {
        Ok(())
    }
}
pub fn validate_id(id: &str) -> Result<(), Error> {
    if id.len() == 66 && id.starts_with("e_") && id[2..].bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(Error::Invalid)
    }
}

pub fn validate_area(scope: Scope, area: Option<&str>) -> Result<(), Error> {
    if let Some(area) = area
        && (scope != Scope::Meenseek || !AREAS.iter().any(|(id, _)| *id == area))
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
