//! Request-time career discovery and comparison. Source facts stay with their owners.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    harness::{
        CareerOutputSurface, DataOwner, HarnessError, HarnessResult, PolicyConfiguration,
        SourceVersion, StoredSourceState, byte_digest, decode_current_json, serialized_digest,
    },
    redaction::redact_secrets,
    search_text::{expanded_query_terms, expanded_search_text},
};

pub const MAX_CAREER_SOURCES: usize = 1_000;
pub const MAX_CAREER_SOURCE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_CAREER_TOTAL_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_CAREER_DEPTH: usize = 32;

fn invalid(message: impl Into<String>) -> HarnessError {
    HarnessError::InvalidRequest(message.into())
}

pub fn digest<T: Serialize>(value: &T) -> HarnessResult<String> {
    serialized_digest(value)
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CareerAxis {
    Domain,
    Technical,
    Competency,
    Role,
    Problem,
    Constraint,
    Decision,
    Implementation,
    Validation,
    Result,
    Learning,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CareerAxisQuery {
    pub axis: CareerAxis,
    pub query: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CareerDiscoveryRequest {
    pub original_request: String,
    pub surface: CareerOutputSurface,
    pub approved_scopes: Vec<String>,
    pub axis_queries: Vec<CareerAxisQuery>,
}

impl CareerDiscoveryRequest {
    pub fn validate(&self) -> HarnessResult<()> {
        if self.original_request.trim().is_empty()
            || self.original_request.len() > 16_384
            || self.approved_scopes.is_empty()
            || self.approved_scopes.len() > 64
            || self.axis_queries.len() > 64
        {
            return Err(invalid(
                "career discovery inputs are empty or exceed limits",
            ));
        }
        let scopes: BTreeSet<_> = self.approved_scopes.iter().collect();
        if scopes.len() != self.approved_scopes.len()
            || !scopes.contains(&"personal".to_owned())
            || scopes.iter().any(|s| !valid_scope(s))
        {
            return Err(invalid(
                "career scopes must be unique exact scopes including personal",
            ));
        }
        let mut queries = BTreeSet::new();
        for q in &self.axis_queries {
            if q.query.trim().is_empty()
                || q.query.chars().count() > 120
                || expanded_query_terms(&q.query).is_empty()
                || !queries.insert((q.axis, q.query.clone()))
            {
                return Err(invalid("career axis queries must be unique bounded text"));
            }
        }
        Ok(())
    }
}

fn valid_scope(scope: &str) -> bool {
    scope == "personal"
        || scope.strip_prefix("work/").is_some_and(|s| {
            !s.is_empty()
                && s.len() <= 64
                && s.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        })
}

pub fn scope_for(path: &str) -> HarnessResult<String> {
    let relative = path
        .strip_prefix("vault/")
        .ok_or_else(|| invalid("career source must be a scoped logical path"))?;
    let parts: Vec<_> = relative.split('/').collect();
    if path.len() > 4_096
        || !path.ends_with(".md")
        || parts.len() > 32
        || parts.iter().any(|p| {
            p.is_empty()
                || p.starts_with('.')
                || p.contains('\\')
                || p.chars().any(char::is_control)
                || matches!(*p, "journal" | "raw")
        })
    {
        return Err(invalid("invalid or restricted career source path"));
    }
    match parts.as_slice() {
        ["personal", _, ..] => Ok("personal".into()),
        ["work", company, _, ..] if valid_scope(&format!("work/{company}")) => {
            Ok(format!("work/{company}"))
        }
        _ => Err(invalid(
            "career sources must belong to personal or one company",
        )),
    }
}

pub fn owner_allows(owner: &DataOwner, path: &str) -> HarnessResult<bool> {
    scope_for(path)?;
    Ok(owner.career_evidence_roots()?.iter().any(|r| {
        path == r
            || path
                .strip_prefix(r)
                .is_some_and(|tail| tail.starts_with('/'))
    }))
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CareerRouteKind {
    Index,
    Fact,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CareerRoute {
    pub owner: DataOwner,
    pub kind: CareerRouteKind,
    pub path: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supporting_sources: Vec<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Routes {
    routes: Vec<CareerRoute>,
}

/// Only explicitly declared routing data is followed. Ordinary links are not grants.
pub fn parse_routes(text: &str) -> HarnessResult<Vec<CareerRoute>> {
    let mut block = None;
    let mut inside = false;
    let mut body = String::new();
    for line in text.lines() {
        if line == "```career-routes" {
            if inside || block.is_some() {
                return Err(invalid("duplicate career-routes block"));
            }
            inside = true;
        } else if inside && line == "```" {
            inside = false;
            block = Some(std::mem::take(&mut body));
        } else if inside {
            body.push_str(line);
            body.push('\n');
        }
    }
    if inside {
        return Err(invalid("unterminated career-routes block"));
    }
    let content = block.ok_or_else(|| invalid("declared career-routes block is missing"))?;
    let parsed: Routes = decode_current_json(content.as_bytes(), "career routes")?;
    if parsed.routes.is_empty() || parsed.routes.len() > MAX_CAREER_SOURCES {
        return Err(invalid("empty or oversized career routes"));
    }
    let mut paths = BTreeSet::new();
    for route in &parsed.routes {
        if route.label.trim().is_empty()
            || route.label.len() > 640
            || !paths.insert(route.path.clone())
            || !owner_allows(&route.owner, &route.path)?
            || route.supporting_sources.len() > 64
        {
            return Err(invalid("career route is duplicated or outside its owner"));
        }
        let mut supporting = BTreeSet::new();
        for path in &route.supporting_sources {
            if route.kind != CareerRouteKind::Fact
                || path == &route.path
                || !supporting.insert(path)
                || !owner_allows(&route.owner, path)?
            {
                return Err(invalid(
                    "supporting source is duplicated or outside its owner",
                ));
            }
        }
    }
    Ok(parsed.routes)
}

#[derive(Clone, Debug)]
pub struct CareerOriginal {
    pub path: String,
    pub content: String,
    pub version: Option<SourceVersion>,
}

impl CareerOriginal {
    pub fn binding(&self) -> HarnessResult<CareerSourceBinding> {
        let content_digest = byte_digest(self.content.as_bytes());
        if self.content.len() > MAX_CAREER_SOURCE_BYTES {
            return Err(invalid("career source exceeds byte limit"));
        }
        if let Some(version) = &self.version {
            version.validate()?;
            if version.logical_path != self.path
                || !matches!(&version.state, StoredSourceState::Live {content_digest: sha, ..} if sha == &content_digest)
            {
                return Err(invalid(
                    "career source identity does not match original bytes",
                ));
            }
        }
        Ok(CareerSourceBinding {
            path: self.path.clone(),
            content_digest,
            version: self.version.clone(),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CareerSourceBinding {
    pub path: String,
    pub content_digest: String,
    pub version: Option<SourceVersion>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CareerMatch {
    pub axis: CareerAxis,
    pub query: String,
    pub path: String,
    pub matched_terms: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CareerCandidate {
    /// Existing fact-source identity, not a permanent generated capsule identifier.
    pub path: String,
    pub owner: DataOwner,
    pub label: String,
    pub sources: Vec<CareerSourceBinding>,
    pub matches: Vec<CareerMatch>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CareerInventoryIssue {
    pub path: String,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CareerInventory {
    pub store_id: Option<String>,
    pub policy_configuration_digest: String,
    pub request: CareerDiscoveryRequest,
    pub router_sources: Vec<CareerSourceBinding>,
    pub candidates: Vec<CareerCandidate>,
    pub scope_excluded: Vec<CareerRoute>,
    pub incomplete: Vec<CareerInventoryIssue>,
    pub inventory_digest: String,
}

/// Byte offsets into the exact UTF-8 original, never a generated paraphrase.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CareerSpan {
    pub start: usize,
    pub end: usize,
}

impl CareerSpan {
    fn text<'a>(&self, original: &'a str) -> HarnessResult<&'a str> {
        original
            .get(self.start..self.end)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| invalid("career span must identify nonempty original UTF-8 bytes"))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
/// Constraint means an evidence-selection constraint (role, period, technology or work
/// conditions). Non-evidence output constraints and background remain Context spans;
/// surface validation and independent review must still check them against exact output.
pub enum CareerRequirementKind {
    Requirement,
    Constraint,
    Context,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CareerCoverage {
    Direct,
    Partial,
    Gap,
    Unverified,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CareerRequirement {
    pub id: String,
    pub original_span: CareerSpan,
    pub kind: CareerRequirementKind,
    pub coverage: CareerCoverage,
    pub material: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CareerLocator {
    pub path: String,
    pub original_span: CareerSpan,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CareerFacet {
    pub axis: CareerAxis,
    pub original: CareerLocator,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CareerDisposition {
    Selected,
    Excluded,
    Unresolved,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CareerDecision {
    pub requirement_id: String,
    pub candidate_path: String,
    pub disposition: CareerDisposition,
    pub reason: String,
    pub facets: Vec<CareerFacet>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CareerPlacement {
    pub claim_id: String,
    pub candidate_path: String,
    pub requirement_ids: Vec<String>,
    pub source_locators: Vec<CareerLocator>,
    pub target: String,
    pub slot: String,
}

/// Transient EvidenceAtoms/ClaimTokens/Capsules share existing fact identities.
/// Semantic relevance and truth still require an independent reviewer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CareerComparison {
    pub inventory: CareerInventory,
    pub requirements: Vec<CareerRequirement>,
    pub decisions: Vec<CareerDecision>,
    pub placements: Vec<CareerPlacement>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CareerReviewView {
    pub comparison_digest: String,
    pub inventory_digest: String,
    pub original_request: String,
    pub requirements: Vec<CareerRequirement>,
    pub decisions: Vec<CareerDecision>,
    pub placements: Vec<CareerPlacement>,
    pub total_candidates: usize,
    pub compared_foreign_owners: Vec<DataOwner>,
    pub scope_excluded: Vec<String>,
}

impl CareerComparison {
    pub fn review_view(&self, owner: &DataOwner) -> HarnessResult<CareerReviewView> {
        let paths = self
            .inventory
            .candidates
            .iter()
            .filter(|c| &c.owner == owner)
            .map(|c| c.path.as_str())
            .collect::<BTreeSet<_>>();
        Ok(CareerReviewView {
            comparison_digest: digest(self)?,
            inventory_digest: self.inventory.inventory_digest.clone(),
            original_request: self.inventory.request.original_request.clone(),
            requirements: self.requirements.clone(),
            decisions: self
                .decisions
                .iter()
                .filter(|d| paths.contains(d.candidate_path.as_str()))
                .cloned()
                .collect(),
            placements: self
                .placements
                .iter()
                .filter(|p| {
                    *owner == DataOwner::Personal || paths.contains(p.candidate_path.as_str())
                })
                .cloned()
                .collect(),
            total_candidates: self.inventory.candidates.len(),
            compared_foreign_owners: self.foreign_owners().into_iter().collect(),
            scope_excluded: self
                .inventory
                .scope_excluded
                .iter()
                .map(|r| r.path.clone())
                .collect(),
        })
    }
    pub fn sources(&self) -> Vec<CareerSourceBinding> {
        let mut sources = BTreeMap::new();
        for s in self
            .inventory
            .router_sources
            .iter()
            .chain(self.inventory.candidates.iter().flat_map(|c| &c.sources))
        {
            sources.insert(s.path.clone(), s.clone());
        }
        sources.into_values().collect()
    }

    pub fn foreign_owners(&self) -> BTreeSet<DataOwner> {
        self.inventory
            .candidates
            .iter()
            .map(|c| c.owner.clone())
            .filter(|o| *o != DataOwner::Personal)
            .collect()
    }

    pub fn owner_source_paths(&self, owner: &DataOwner) -> Vec<String> {
        self.inventory
            .candidates
            .iter()
            .filter(|c| &c.owner == owner)
            .flat_map(|c| c.sources.iter().map(|s| s.path.clone()))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn validate_structure(&self, finalization: bool) -> HarnessResult<()> {
        self.inventory.require_complete()?;
        if self.requirements.is_empty()
            || self.requirements.len() > 64
            || self.decisions.len() > 64_000
            || self.placements.len() > 64
        {
            return Err(invalid(
                "career comparison exceeds explicit cardinality limits",
            ));
        }
        let request = &self.inventory.request.original_request;
        let candidates = self
            .inventory
            .candidates
            .iter()
            .map(|c| (c.path.as_str(), c))
            .collect::<BTreeMap<_, _>>();
        if candidates.len() != self.inventory.candidates.len() {
            return Err(invalid("duplicate career candidate"));
        }
        let mut requirements = BTreeMap::new();
        let mut end = 0;
        let mut required = 0;
        for r in &self.requirements {
            r.original_span.text(request)?;
            if !valid_id(&r.id)
                || requirements.insert(r.id.as_str(), r).is_some()
                || r.original_span.start < end
                || !request
                    .get(end..r.original_span.start)
                    .is_some_and(|s| s.trim().is_empty())
            {
                return Err(invalid(
                    "requirements must uniquely cover the original request in order",
                ));
            }
            end = r.original_span.end;
            if r.kind == CareerRequirementKind::Context {
                if r.material || r.coverage != CareerCoverage::Unverified {
                    return Err(invalid("context is not an evidence requirement"));
                }
            } else {
                required += 1;
                if finalization && r.material && r.coverage == CareerCoverage::Unverified {
                    return Err(invalid(
                        "material unverified requirements block finalization",
                    ));
                }
            }
        }
        if required == 0 || !request.get(end..).is_some_and(|s| s.trim().is_empty()) {
            return Err(invalid("original request clauses were omitted"));
        }
        let mut rows = BTreeMap::new();
        for d in &self.decisions {
            let r = requirements
                .get(d.requirement_id.as_str())
                .ok_or_else(|| invalid("decision requirement is unknown"))?;
            let c = candidates
                .get(d.candidate_path.as_str())
                .ok_or_else(|| invalid("decision candidate is unknown"))?;
            if r.kind == CareerRequirementKind::Context
                || d.reason.trim().is_empty()
                || d.reason.len() > 2_048
                || rows
                    .insert((d.requirement_id.as_str(), d.candidate_path.as_str()), d)
                    .is_some()
                || d.facets.len() > 64
            {
                return Err(invalid(
                    "career decisions must be unique, bounded and explained",
                ));
            }
            if d.disposition == CareerDisposition::Unresolved {
                if finalization {
                    return Err(invalid("unresolved comparisons block finalization"));
                }
            } else if d.facets.is_empty() {
                return Err(invalid(
                    "selected and excluded decisions require original evidence",
                ));
            }
            for f in &d.facets {
                validate_locator_binding(c, &f.original)?;
            }
        }
        for r in self
            .requirements
            .iter()
            .filter(|r| r.kind != CareerRequirementKind::Context)
        {
            let mut selected = 0;
            let mut unresolved = 0;
            for c in &self.inventory.candidates {
                let d = rows.get(&(r.id.as_str(), c.path.as_str())).ok_or_else(|| {
                    invalid("every requirement must compare every approved candidate")
                })?;
                selected += usize::from(d.disposition == CareerDisposition::Selected);
                unresolved += usize::from(d.disposition == CareerDisposition::Unresolved);
            }
            match r.coverage {
                CareerCoverage::Direct | CareerCoverage::Partial if selected == 0 => {
                    return Err(invalid("covered requirement has no selected experience"));
                }
                CareerCoverage::Gap if selected > 0 || unresolved > 0 => {
                    return Err(invalid(
                        "a scoped gap requires complete comparisons with no selected or unresolved candidate",
                    ));
                }
                _ => {}
            }
        }
        let mut claims = BTreeSet::new();
        for p in &self.placements {
            let c = candidates
                .get(p.candidate_path.as_str())
                .ok_or_else(|| invalid("placement candidate is unknown"))?;
            if !valid_id(&p.claim_id)
                || !claims.insert(&p.claim_id)
                || p.target.trim().is_empty()
                || p.target.len() > 4_096
                || p.slot.trim().is_empty()
                || p.slot.len() > 640
                || p.requirement_ids.is_empty()
                || p.requirement_ids.len() > 64
                || p.source_locators.is_empty()
                || p.source_locators.len() > 64
            {
                return Err(invalid(
                    "claim placements must be uniquely bound to a target, slot and original evidence",
                ));
            }
            let mut linked = BTreeSet::new();
            for id in &p.requirement_ids {
                if !linked.insert(id)
                    || !rows
                        .get(&(id.as_str(), p.candidate_path.as_str()))
                        .is_some_and(|d| d.disposition == CareerDisposition::Selected)
                {
                    return Err(invalid(
                        "placements may only use experiences selected for their requirement",
                    ));
                }
            }
            for l in &p.source_locators {
                validate_locator_binding(c, l)?;
            }
        }
        if finalization {
            for decision in self
                .decisions
                .iter()
                .filter(|d| d.disposition == CareerDisposition::Selected)
            {
                if !self.placements.iter().any(|p| {
                    p.candidate_path == decision.candidate_path
                        && p.requirement_ids.contains(&decision.requirement_id)
                }) {
                    return Err(invalid(
                        "every selected requirement/experience must appear in the final claim placements",
                    ));
                }
            }
        }
        if finalization && self.placements.is_empty() {
            return Err(invalid("finalization requires claim placements"));
        }
        Ok(())
    }

    /// Current inventory is reconstructed by a trusted adapter, not supplied by the caller.
    pub fn validate_current(
        &self,
        current: &CareerInventory,
        originals: &BTreeMap<String, CareerOriginal>,
        finalization: bool,
    ) -> HarnessResult<()> {
        self.validate_structure(finalization)?;
        if &self.inventory != current {
            return Err(invalid("career inventory is stale, changed or incomplete"));
        }
        for binding in self.sources() {
            if originals
                .get(&binding.path)
                .map(CareerOriginal::binding)
                .transpose()?
                .as_ref()
                != Some(&binding)
            {
                return Err(invalid(
                    "comparison original bytes do not match the current source identity",
                ));
            }
        }
        for l in self
            .decisions
            .iter()
            .flat_map(|d| d.facets.iter().map(|f| &f.original))
            .chain(self.placements.iter().flat_map(|p| &p.source_locators))
        {
            let original = originals
                .get(&l.path)
                .ok_or_else(|| invalid("locator original is unavailable"))?;
            l.original_span.text(&original.content)?;
        }
        Ok(())
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

fn validate_locator_binding(
    candidate: &CareerCandidate,
    locator: &CareerLocator,
) -> HarnessResult<()> {
    if locator.original_span.start >= locator.original_span.end
        || !candidate.sources.iter().any(|s| s.path == locator.path)
    {
        return Err(invalid(
            "career locator must belong to the candidate's declared fact originals",
        ));
    }
    Ok(())
}

impl CareerInventory {
    pub fn content_digest(&self) -> HarnessResult<String> {
        let mut normalized = self.clone();
        normalized.inventory_digest.clear();
        digest(&normalized)
    }
    pub fn require_complete(&self) -> HarnessResult<()> {
        self.request.validate()?;
        if self.inventory_digest != self.content_digest()?
            || !self.incomplete.is_empty()
            || self.candidates.is_empty()
        {
            return Err(invalid(
                "career inventory is incomplete or its digest has changed",
            ));
        }
        Ok(())
    }
}

/// Pure traversal; adapters fetch each wave as one bounded original-byte batch.
pub fn discover<F>(
    configuration: &PolicyConfiguration,
    store_id: Option<String>,
    request: CareerDiscoveryRequest,
    mut read: F,
) -> HarnessResult<CareerInventory>
where
    F: FnMut(&[String]) -> HarnessResult<Vec<CareerOriginal>>,
{
    request.validate()?;
    let roots = configuration.career_source_roots()?;
    let root_originals = read(&roots)?;
    if root_originals.len() != roots.len() {
        return Err(invalid("canonical career routers are missing"));
    }
    let mut inventory = CareerInventory {
        store_id,
        policy_configuration_digest: configuration.digest().into(),
        request,
        router_sources: Vec::new(),
        candidates: Vec::new(),
        scope_excluded: Vec::new(),
        incomplete: Vec::new(),
        inventory_digest: String::new(),
    };
    let mut visited = BTreeSet::new();
    let mut original_cache = BTreeMap::new();
    let mut total_bytes = 0;
    let mut queue = Vec::new();
    let mut index_links = BTreeMap::new();
    for (position, path) in roots.iter().enumerate() {
        let original = root_originals
            .iter()
            .find(|o| &o.path == path)
            .ok_or_else(|| invalid("career root response path mismatch"))?;
        total_bytes += original.content.len();
        inventory.router_sources.push(original.binding()?);
        let routes = parse_routes(&original.content)?;
        index_links.insert(
            path.clone(),
            routes
                .iter()
                .filter(|r| r.kind == CareerRouteKind::Index)
                .map(|r| r.path.clone())
                .collect::<Vec<_>>(),
        );
        if position == 1 && routes.iter().any(|r| r.owner != DataOwner::Personal) {
            return Err(invalid(
                "source map auxiliary routes must belong to primary personal",
            ));
        }
        queue.extend(
            routes
                .into_iter()
                .map(|r| (r, BTreeSet::from([path.clone()]))),
        );
        visited.insert(path.clone());
        original_cache.insert(path.clone(), original.clone());
    }
    for _ in 0..MAX_CAREER_DEPTH {
        if queue.is_empty() {
            break;
        }
        let mut pending = Vec::new();
        let mut wanted = BTreeSet::new();
        for (route, ancestors) in std::mem::take(&mut queue) {
            if !inventory
                .request
                .approved_scopes
                .contains(&scope_for(&route.path)?)
            {
                inventory.scope_excluded.push(route);
                continue;
            }
            if ancestors.contains(&route.path) {
                inventory.incomplete.push(CareerInventoryIssue {
                    path: route.path,
                    reason: "index cycle".into(),
                });
                continue;
            }
            for path in std::iter::once(&route.path).chain(&route.supporting_sources) {
                if !original_cache.contains_key(path) {
                    wanted.insert(path.clone());
                }
            }
            pending.push((route, ancestors));
        }
        if original_cache.len() + wanted.len() > MAX_CAREER_SOURCES {
            inventory.incomplete.push(CareerInventoryIssue {
                path: String::new(),
                reason: "source-count limit".into(),
            });
            break;
        }
        if !wanted.is_empty() {
            let originals = read(&wanted.iter().cloned().collect::<Vec<_>>())?;
            let mut received = BTreeSet::new();
            for original in originals {
                if !wanted.contains(&original.path) || !received.insert(original.path.clone()) {
                    return Err(invalid("unexpected or duplicated original response"));
                }
                original.binding()?;
                total_bytes += original.content.len();
                if total_bytes > MAX_CAREER_TOTAL_BYTES {
                    return Err(invalid("career aggregate byte limit"));
                }
                original_cache.insert(original.path.clone(), original);
            }
        }
        for (route, mut ancestors) in pending {
            let paths: Vec<_> = std::iter::once(&route.path)
                .chain(&route.supporting_sources)
                .collect();
            if let Some(path) = paths
                .iter()
                .find(|p| !original_cache.contains_key(p.as_str()))
            {
                inventory.incomplete.push(CareerInventoryIssue {
                    path: (*path).clone(),
                    reason: "declared original unavailable".into(),
                });
                continue;
            }
            let original = &original_cache[&route.path];
            if route.kind == CareerRouteKind::Index {
                if !visited.insert(route.path.clone()) {
                    continue;
                }
                inventory.router_sources.push(original.binding()?);
                ancestors.insert(route.path.clone());
                match parse_routes(&original.content) {
                    Ok(children) => {
                        if children.iter().any(|r| r.owner != route.owner) {
                            return Err(invalid("local case index changed its source owner"));
                        }
                        index_links.insert(
                            route.path.clone(),
                            children
                                .iter()
                                .filter(|r| r.kind == CareerRouteKind::Index)
                                .map(|r| r.path.clone())
                                .collect::<Vec<_>>(),
                        );
                        queue.extend(children.into_iter().map(|r| (r, ancestors.clone())));
                    }
                    Err(error) => inventory.incomplete.push(CareerInventoryIssue {
                        path: route.path,
                        reason: error.to_string(),
                    }),
                }
            } else {
                if inventory.candidates.iter().any(|c| c.path == route.path) {
                    return Err(invalid("duplicate career fact route"));
                }
                let mut sources = Vec::new();
                let mut matches = Vec::new();
                for path in paths {
                    let original = &original_cache[path];
                    sources.push(original.binding()?);
                    let searchable = expanded_search_text(&[&redact_secrets(&original.content)]);
                    for axis in &inventory.request.axis_queries {
                        let groups = expanded_query_terms(&axis.query);
                        if groups.iter().all(|g| {
                            g.iter()
                                .any(|t| searchable.split_whitespace().any(|s| s == t))
                        }) {
                            let matched_terms = groups
                                .into_iter()
                                .flatten()
                                .filter(|t| searchable.split_whitespace().any(|s| s == t))
                                .collect();
                            matches.push(CareerMatch {
                                axis: axis.axis,
                                query: axis.query.clone(),
                                path: path.clone(),
                                matched_terms,
                            });
                        }
                    }
                }
                sources.sort_by(|a, b| a.path.cmp(&b.path));
                inventory.candidates.push(CareerCandidate {
                    path: route.path,
                    owner: route.owner,
                    label: redact_secrets(&route.label),
                    sources,
                    matches,
                });
            }
        }
    }
    if !queue.is_empty() {
        inventory.incomplete.push(CareerInventoryIssue {
            path: String::new(),
            reason: "index-depth limit".into(),
        });
    }
    let mut complete = BTreeSet::new();
    for path in index_links.keys() {
        if index_cycle(&index_links, path, &mut BTreeSet::new(), &mut complete) {
            inventory.incomplete.push(CareerInventoryIssue {
                path: path.clone(),
                reason: "index cycle".into(),
            });
            break;
        }
    }
    inventory.router_sources.sort_by(|a, b| a.path.cmp(&b.path));
    inventory.candidates.sort_by(|a, b| a.path.cmp(&b.path));
    inventory.scope_excluded.sort_by(|a, b| a.path.cmp(&b.path));
    inventory.incomplete.sort_by(|a, b| a.path.cmp(&b.path));
    inventory.inventory_digest = inventory.content_digest()?;
    Ok(inventory)
}

fn index_cycle(
    edges: &BTreeMap<String, Vec<String>>,
    path: &str,
    active: &mut BTreeSet<String>,
    complete: &mut BTreeSet<String>,
) -> bool {
    if complete.contains(path) {
        return false;
    }
    if !active.insert(path.into()) {
        return true;
    }
    if edges.get(path).is_some_and(|links| {
        links
            .iter()
            .any(|p| index_cycle(edges, p, active, complete))
    }) {
        return true;
    }
    active.remove(path);
    complete.insert(path.into());
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (
        PolicyConfiguration,
        BTreeMap<String, CareerOriginal>,
        CareerDiscoveryRequest,
    ) {
        let mut settings: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/policy-settings.json")).unwrap();
        settings["documents"].as_array_mut().unwrap().extend([
            serde_json::json!({"key":"CareerRoutes","id":"synthetic-career-routes","path":"vault/personal/writing/routes.md","project_entrypoint":false,"dependencies":[]}),
            serde_json::json!({"key":"CareerSources","id":"synthetic-career-sources","path":"vault/personal/writing/sources.md","project_entrypoint":false,"dependencies":[]}),
        ]);
        settings["career"] =
            serde_json::json!({"experience_routes":"CareerRoutes","source_map":"CareerSources"});
        let configuration =
            PolicyConfiguration::from_bytes(&serde_json::to_vec(&settings).unwrap()).unwrap();
        let mut originals = BTreeMap::new();
        let mut insert = |path: &str, content: &str| {
            originals.insert(
                path.into(),
                CareerOriginal {
                    path: path.into(),
                    content: content.into(),
                    version: None,
                },
            );
        };
        insert(
            "vault/personal/writing/routes.md",
            "# Experience routes\n```career-routes\n{\"routes\": [{\"owner\": {\"kind\": \"personal-project\", \"project\": \"sample\"}, \"kind\": \"index\", \"path\": \"vault/personal/projects/sample/index.md\", \"label\": \"sample\"}, {\"owner\": {\"kind\": \"company\", \"company\": \"example\"}, \"kind\": \"fact\", \"path\": \"vault/work/example/experience/api.md\", \"label\": \"API\"}]}\n```\n",
        );
        insert(
            "vault/personal/writing/sources.md",
            "# Auxiliary\n```career-routes\n{\"routes\": [{\"owner\": {\"kind\": \"personal\"}, \"kind\": \"fact\", \"path\": \"vault/personal/profile.md\", \"label\": \"profile\"}]}\n```\n",
        );
        insert(
            "vault/personal/projects/sample/index.md",
            "# Cases\n[unregistered](other.md)\n```career-routes\n{\"routes\": [{\"owner\": {\"kind\": \"personal-project\", \"project\": \"sample\"}, \"kind\": \"fact\", \"path\": \"vault/personal/projects/sample/mentoring.md\", \"label\": \"mentoring\"}]}\n```\n",
        );
        insert(
            "vault/personal/projects/sample/mentoring.md",
            "# Mentoring\nTogether we discussed design and reduced contribution burden.\n",
        );
        insert(
            "vault/personal/profile.md",
            "# Profile\nAward and learning.\n",
        );
        insert(
            "vault/work/example/experience/api.md",
            "# API\nTechnical implementation.\n",
        );
        let request = CareerDiscoveryRequest {
            original_request: "타인 또는 공동체를 위한 노력과 배운 점".into(),
            surface: CareerOutputSurface::General,
            approved_scopes: vec!["personal".into(), "work/example".into()],
            axis_queries: vec![CareerAxisQuery {
                axis: CareerAxis::Competency,
                query: "community".into(),
            }],
        };
        (configuration, originals, request)
    }

    #[test]
    fn zero_matches_preserve_the_inventory_and_read_by_wave() {
        let (configuration, originals, request) = fixture();
        let mut waves = Vec::new();
        let inventory = discover(&configuration, None, request, |paths| {
            waves.push(paths.to_vec());
            Ok(paths
                .iter()
                .filter_map(|p| originals.get(p).cloned())
                .collect())
        })
        .unwrap();
        inventory.require_complete().unwrap();
        assert_eq!(inventory.candidates.len(), 3);
        assert!(inventory.candidates.iter().all(|c| c.matches.is_empty()));
        assert_eq!(waves.len(), 3); // two routers, one shared level, one nested case level
        assert!(!waves.iter().flatten().any(|p| p.ends_with("other.md")));
    }

    fn bounded_fixture(
        fact_count: usize,
    ) -> (
        PolicyConfiguration,
        BTreeMap<String, CareerOriginal>,
        CareerDiscoveryRequest,
    ) {
        let (configuration, mut originals, mut request) = fixture();
        originals.retain(|path, _| {
            path.starts_with("vault/personal/writing/") || path == "vault/personal/profile.md"
        });
        request.approved_scopes = vec!["personal".into()];
        request.axis_queries.clear();
        let routes = (0..fact_count)
            .map(|i| {
                let path = format!("vault/personal/projects/sample/fact-{i:04}.md");
                originals.insert(
                    path.clone(),
                    CareerOriginal {
                        path: path.clone(),
                        content: "# Synthetic fact\n".into(),
                        version: None,
                    },
                );
                CareerRoute {
                    owner: DataOwner::PersonalProject {
                        project: "sample".into(),
                    },
                    kind: CareerRouteKind::Fact,
                    path,
                    label: "synthetic fact".into(),
                    supporting_sources: vec![],
                }
            })
            .collect();
        originals
            .get_mut("vault/personal/writing/routes.md")
            .unwrap()
            .content = routes_block(routes);
        (configuration, originals, request)
    }

    fn routes_block(routes: Vec<CareerRoute>) -> String {
        format!(
            "```career-routes\n{}\n```\n",
            serde_json::to_string(&Routes { routes }).unwrap()
        )
    }

    fn discover_fixture(
        configuration: &PolicyConfiguration,
        originals: &BTreeMap<String, CareerOriginal>,
        request: CareerDiscoveryRequest,
    ) -> HarnessResult<CareerInventory> {
        discover(configuration, None, request, |paths| {
            Ok(paths
                .iter()
                .filter_map(|p| originals.get(p).cloned())
                .collect())
        })
    }

    #[test]
    fn discovery_limit_source_count_preserves_boundary_and_blocks_overflow_before_read() {
        // The inventory also reads two routers and one auxiliary personal fact.
        let fact_count = MAX_CAREER_SOURCES - 3;
        let (configuration, originals, request) = bounded_fixture(fact_count);
        assert_eq!(originals.len(), MAX_CAREER_SOURCES);
        let inventory = discover_fixture(&configuration, &originals, request).unwrap();
        inventory.require_complete().unwrap();
        assert_eq!(inventory.candidates.len(), fact_count + 1);
        assert_eq!(inventory.router_sources.len(), 2);

        let (configuration, originals, request) = bounded_fixture(fact_count + 1);
        let mut reads = Vec::new();
        let inventory = discover(&configuration, None, request, |paths| {
            reads.extend_from_slice(paths);
            Ok(paths
                .iter()
                .filter_map(|p| originals.get(p).cloned())
                .collect())
        })
        .unwrap();
        assert_eq!(reads, configuration.career_source_roots().unwrap());
        assert!(inventory.candidates.is_empty());
        assert!(
            inventory
                .incomplete
                .iter()
                .any(|i| i.reason == "source-count limit")
        );
        assert!(inventory.require_complete().is_err());
    }

    #[test]
    fn discovery_limit_source_bytes_accepts_exact_boundary_and_rejects_one_more_byte() {
        let (configuration, mut originals, request) = bounded_fixture(1);
        let path = "vault/personal/projects/sample/fact-0000.md";
        originals.get_mut(path).unwrap().content = "x".repeat(MAX_CAREER_SOURCE_BYTES);
        let inventory = discover_fixture(&configuration, &originals, request.clone()).unwrap();
        inventory.require_complete().unwrap();
        let candidate = inventory
            .candidates
            .iter()
            .find(|c| c.path == path)
            .unwrap();
        assert_eq!(candidate.sources[0], originals[path].binding().unwrap());

        originals.get_mut(path).unwrap().content.push('x');
        let error = discover_fixture(&configuration, &originals, request).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("career source exceeds byte limit")
        );
    }

    #[test]
    fn discovery_limit_aggregate_bytes_counts_routers_and_rejects_one_more_byte() {
        let fact_count = MAX_CAREER_TOTAL_BYTES / MAX_CAREER_SOURCE_BYTES;
        let (configuration, mut originals, request) = bounded_fixture(fact_count);
        let overhead: usize = originals
            .values()
            .filter(|o| !o.path.contains("/fact-"))
            .map(|o| o.content.len())
            .sum();
        assert!(overhead < MAX_CAREER_SOURCE_BYTES);
        for i in 0..fact_count {
            let path = format!("vault/personal/projects/sample/fact-{i:04}.md");
            let bytes = MAX_CAREER_SOURCE_BYTES - if i == fact_count - 1 { overhead } else { 0 };
            originals.get_mut(&path).unwrap().content = "x".repeat(bytes);
        }
        assert_eq!(
            originals.values().map(|o| o.content.len()).sum::<usize>(),
            MAX_CAREER_TOTAL_BYTES
        );
        let inventory = discover_fixture(&configuration, &originals, request.clone()).unwrap();
        inventory.require_complete().unwrap();
        assert_eq!(inventory.candidates.len(), fact_count + 1);

        let last = format!(
            "vault/personal/projects/sample/fact-{:04}.md",
            fact_count - 1
        );
        originals.get_mut(&last).unwrap().content.push('x');
        assert!(
            originals
                .values()
                .all(|o| o.content.len() <= MAX_CAREER_SOURCE_BYTES)
        );
        let error = discover_fixture(&configuration, &originals, request).unwrap_err();
        assert!(error.to_string().contains("career aggregate byte limit"));
    }

    #[test]
    fn discovery_limit_depth_preserves_last_wave_and_exposes_unread_next_wave() {
        let (configuration, base, request) = bounded_fixture(1);
        for depth in [MAX_CAREER_DEPTH, MAX_CAREER_DEPTH + 1] {
            let mut originals = base.clone();
            let fact = "vault/personal/projects/sample/fact-0000.md";
            for level in 0..depth {
                let parent = if level == 0 {
                    "vault/personal/writing/routes.md".into()
                } else {
                    format!("vault/personal/projects/sample/index-{level:02}.md")
                };
                let last = level == depth - 1;
                let child = if last {
                    fact.into()
                } else {
                    format!("vault/personal/projects/sample/index-{:02}.md", level + 1)
                };
                originals.insert(
                    parent.clone(),
                    CareerOriginal {
                        path: parent,
                        version: None,
                        content: routes_block(vec![CareerRoute {
                            owner: DataOwner::PersonalProject {
                                project: "sample".into(),
                            },
                            kind: if last {
                                CareerRouteKind::Fact
                            } else {
                                CareerRouteKind::Index
                            },
                            path: child,
                            label: "synthetic depth".into(),
                            supporting_sources: vec![],
                        }]),
                    },
                );
            }
            let mut reads = Vec::new();
            let inventory = discover(&configuration, None, request.clone(), |paths| {
                reads.extend_from_slice(paths);
                Ok(paths
                    .iter()
                    .filter_map(|p| originals.get(p).cloned())
                    .collect())
            })
            .unwrap();
            if depth == MAX_CAREER_DEPTH {
                inventory.require_complete().unwrap();
                assert_eq!(inventory.candidates.len(), 2);
                assert!(reads.iter().any(|p| p == fact));
            } else {
                assert!(inventory.require_complete().is_err());
                assert!(
                    inventory
                        .incomplete
                        .iter()
                        .any(|i| i.reason == "index-depth limit")
                );
                assert_eq!(inventory.candidates.len(), 1); // auxiliary fact remains diagnostic evidence
                assert!(!reads.iter().any(|p| p == fact));
            }
        }
    }

    #[test]
    fn multiple_axes_reuse_the_same_fact_without_rewriting_it() {
        let (configuration, originals, mut request) = fixture();
        request.axis_queries = vec![
            CareerAxisQuery {
                axis: CareerAxis::Competency,
                query: "design".into(),
            },
            CareerAxisQuery {
                axis: CareerAxis::Learning,
                query: "burden".into(),
            },
        ];
        let inventory = discover(&configuration, None, request, |paths| {
            Ok(paths
                .iter()
                .filter_map(|p| originals.get(p).cloned())
                .collect())
        })
        .unwrap();
        let mentoring = inventory
            .candidates
            .iter()
            .find(|c| c.path.ends_with("mentoring.md"))
            .unwrap();
        assert_eq!(mentoring.matches.len(), 2);
        assert_eq!(
            mentoring.sources[0],
            originals[&mentoring.path].binding().unwrap()
        );
    }

    #[test]
    fn outside_scope_routes_are_visible_but_never_read() {
        let (configuration, originals, mut request) = fixture();
        request.approved_scopes = vec!["personal".into()];
        let inventory = discover(&configuration, None, request, |paths| {
            assert!(!paths.iter().any(|p| p.starts_with("vault/work/")));
            Ok(paths
                .iter()
                .filter_map(|p| originals.get(p).cloned())
                .collect())
        })
        .unwrap();
        inventory.require_complete().unwrap();
        assert_eq!(inventory.scope_excluded.len(), 1);
        assert_eq!(inventory.candidates.len(), 2);
    }

    #[test]
    fn unavailable_original_and_cycle_cannot_be_complete() {
        let (configuration, mut originals, request) = fixture();
        originals.remove("vault/personal/projects/sample/mentoring.md");
        let inventory = discover(&configuration, None, request.clone(), |paths| {
            Ok(paths
                .iter()
                .filter_map(|p| originals.get(p).cloned())
                .collect())
        })
        .unwrap();
        assert!(inventory.require_complete().is_err());
        originals.get_mut("vault/personal/projects/sample/index.md").unwrap().content = "```career-routes\n{\"routes\": [{\"owner\": {\"kind\": \"personal-project\", \"project\": \"sample\"}, \"kind\": \"index\", \"path\": \"vault/personal/projects/sample/index.md\", \"label\": \"cycle\"}]}\n```\n".into();
        let inventory = discover(&configuration, None, request, |paths| {
            Ok(paths
                .iter()
                .filter_map(|p| originals.get(p).cloned())
                .collect())
        })
        .unwrap();
        assert!(inventory.require_complete().is_err());
        assert!(
            inventory
                .incomplete
                .iter()
                .any(|p| p.reason == "index cycle")
        );
    }

    #[test]
    fn a_local_index_cannot_change_owner_or_invent_fact_sources() {
        let (configuration, mut originals, request) = fixture();
        originals.get_mut("vault/personal/projects/sample/index.md").unwrap().content = "```career-routes\n{\"routes\": [{\"owner\": {\"kind\": \"personal\"}, \"kind\": \"fact\", \"path\": \"vault/personal/profile.md\", \"label\": \"wrong owner\"}]}\n```\n".into();
        assert!(
            discover(&configuration, None, request, |paths| Ok(paths
                .iter()
                .filter_map(|p| originals.get(p).cloned())
                .collect()))
            .is_err()
        );
        assert!(
            !owner_allows(
                &DataOwner::Personal,
                "vault/personal/writing/applications/submitted.md"
            )
            .unwrap()
        );
        assert!(!owner_allows(&DataOwner::Personal, "vault/personal/business/plan.md").unwrap());
        assert!(scope_for("vault/personal/journal/private.md").is_err());
    }

    #[test]
    fn cycles_between_two_independently_routed_indexes_cannot_hide_behind_visited_paths() {
        let (configuration, mut originals, request) = fixture();
        let first = "vault/personal/projects/sample/index.md";
        let second = "vault/personal/projects/sample/second.md";
        let owner = DataOwner::PersonalProject {
            project: "sample".into(),
        };
        let route = |path: &str, kind| CareerRoute {
            owner: owner.clone(),
            kind,
            path: path.into(),
            label: path.into(),
            supporting_sources: vec![],
        };
        let block = |routes| {
            format!(
                "```career-routes\n{}\n```\n",
                serde_json::to_string(&Routes { routes }).unwrap()
            )
        };
        originals
            .get_mut("vault/personal/writing/routes.md")
            .unwrap()
            .content = block(vec![
            route(first, CareerRouteKind::Index),
            route(second, CareerRouteKind::Index),
        ]);
        originals.get_mut(first).unwrap().content =
            block(vec![route(second, CareerRouteKind::Index)]);
        originals.insert(
            second.into(),
            CareerOriginal {
                path: second.into(),
                version: None,
                content: block(vec![
                    route(first, CareerRouteKind::Index),
                    route(
                        "vault/personal/projects/sample/mentoring.md",
                        CareerRouteKind::Fact,
                    ),
                ]),
            },
        );
        let inventory = discover(&configuration, None, request, |paths| {
            Ok(paths
                .iter()
                .filter_map(|p| originals.get(p).cloned())
                .collect())
        })
        .unwrap();
        assert_eq!(
            inventory.candidates.len(),
            2,
            "reachable facts are retained for diagnosis"
        );
        assert!(
            inventory
                .incomplete
                .iter()
                .any(|i| i.reason == "index cycle")
        );
        assert!(inventory.require_complete().is_err());
    }
    fn compared_fixture() -> (CareerComparison, BTreeMap<String, CareerOriginal>) {
        let (configuration, originals, request) = fixture();
        let inventory = discover(&configuration, None, request, |paths| {
            Ok(paths
                .iter()
                .filter_map(|p| originals.get(p).cloned())
                .collect())
        })
        .unwrap();
        let selected = "vault/personal/projects/sample/mentoring.md";
        let decisions = inventory
            .candidates
            .iter()
            .map(|c| CareerDecision {
                requirement_id: "community".into(),
                candidate_path: c.path.clone(),
                disposition: if c.path == selected {
                    CareerDisposition::Selected
                } else {
                    CareerDisposition::Excluded
                },
                reason: if c.path == selected {
                    "Design conversations supported other contributors"
                } else {
                    "This original does not describe helping contributors"
                }
                .into(),
                facets: vec![CareerFacet {
                    axis: CareerAxis::Competency,
                    original: CareerLocator {
                        path: c.path.clone(),
                        original_span: CareerSpan { start: 0, end: 10 },
                    },
                }],
            })
            .collect();
        let requirements = vec![CareerRequirement {
            id: "community".into(),
            original_span: CareerSpan {
                start: 0,
                end: inventory.request.original_request.len(),
            },
            kind: CareerRequirementKind::Requirement,
            coverage: CareerCoverage::Direct,
            material: true,
        }];
        let placements = vec![CareerPlacement {
            claim_id: "help-others".into(),
            candidate_path: selected.into(),
            requirement_ids: vec!["community".into()],
            source_locators: vec![CareerLocator {
                path: selected.into(),
                original_span: CareerSpan { start: 12, end: 70 },
            }],
            target: "answer.md".into(),
            slot: "community effort".into(),
        }];
        (
            CareerComparison {
                inventory,
                requirements,
                decisions,
                placements,
            },
            originals,
        )
    }

    #[test]
    fn comparison_rejects_omitted_cases_forged_inventory_and_stale_originals() {
        let (comparison, originals) = compared_fixture();
        comparison
            .validate_current(&comparison.inventory, &originals, true)
            .unwrap();
        let mut omitted = comparison.clone();
        omitted.decisions.pop();
        assert!(omitted.validate_structure(true).is_err());
        let mut forged = comparison.clone();
        forged.inventory.candidates.pop();
        forged.decisions.retain(|d| {
            forged
                .inventory
                .candidates
                .iter()
                .any(|c| c.path == d.candidate_path)
        });
        forged.inventory.inventory_digest = forged.inventory.content_digest().unwrap();
        assert!(
            forged
                .validate_current(&comparison.inventory, &originals, true)
                .is_err()
        );
        let mut stale = comparison.inventory.clone();
        stale.router_sources[0].content_digest = "0".repeat(64);
        stale.inventory_digest = stale.content_digest().unwrap();
        assert!(
            comparison
                .validate_current(&stale, &originals, true)
                .is_err()
        );
    }

    #[test]
    fn request_omission_unresolved_and_borrowed_evidence_cannot_finalize() {
        let (comparison, originals) = compared_fixture();
        let mut omitted = comparison.clone();
        omitted.requirements[0].original_span.end -= 3;
        assert!(omitted.validate_structure(true).is_err());
        let mut unresolved = comparison.clone();
        unresolved.decisions[0].disposition = CareerDisposition::Unresolved;
        assert!(unresolved.validate_structure(false).is_ok());
        assert!(unresolved.validate_structure(true).is_err());
        let mut borrowed = comparison.clone();
        borrowed.placements[0].source_locators[0].path =
            "vault/personal/writing/submitted.md".into();
        assert!(borrowed.validate_structure(true).is_err());
        let mut locator = comparison.clone();
        locator.placements[0].source_locators[0].original_span.end = usize::MAX;
        assert!(
            locator
                .validate_current(&comparison.inventory, &originals, true)
                .is_err()
        );
        let mut gap = comparison;
        gap.requirements[0].coverage = CareerCoverage::Gap;
        assert!(gap.validate_structure(true).is_err());
    }

    #[test]
    fn role_view_preserves_shared_identity_without_foreign_decisions_or_raw_text() {
        let (comparison, _) = compared_fixture();
        let view = comparison.review_view(&DataOwner::Personal).unwrap();
        assert_eq!(view.total_candidates, 3);
        assert_eq!(view.decisions.len(), 1);
        assert_eq!(view.compared_foreign_owners.len(), 2);
        assert_eq!(view.placements, comparison.placements);
        assert_eq!(view.comparison_digest, digest(&comparison).unwrap());
    }

    #[test]
    fn compound_questions_need_each_selected_requirement_placed_and_keep_format_context() {
        let (mut comparison, originals) = compared_fixture();
        comparison.inventory.request.original_request =
            "community\nlearning\n600 characters".into();
        comparison.inventory.inventory_digest = comparison.inventory.content_digest().unwrap();
        comparison.requirements[0].original_span.end = 9;
        comparison.requirements.extend([
            CareerRequirement {
                id: "learning".into(),
                original_span: CareerSpan { start: 10, end: 18 },
                kind: CareerRequirementKind::Requirement,
                coverage: CareerCoverage::Direct,
                material: true,
            },
            CareerRequirement {
                id: "format".into(),
                original_span: CareerSpan { start: 19, end: 33 },
                kind: CareerRequirementKind::Context,
                coverage: CareerCoverage::Unverified,
                material: false,
            },
        ]);
        let learning = comparison
            .decisions
            .iter()
            .cloned()
            .map(|mut d| {
                d.requirement_id = "learning".into();
                d
            })
            .collect::<Vec<_>>();
        comparison.decisions.extend(learning);
        assert!(
            comparison.validate_structure(true).is_err(),
            "one part cannot be selected then omitted from the artifact"
        );
        comparison.placements[0]
            .requirement_ids
            .push("learning".into());
        comparison
            .validate_current(&comparison.inventory, &originals, true)
            .unwrap();
        let view = comparison.review_view(&DataOwner::Personal).unwrap();
        assert_eq!(view.original_request, "community\nlearning\n600 characters");
        assert_eq!(
            view.requirements.last().unwrap().kind,
            CareerRequirementKind::Context
        );
    }

    #[test]
    fn checked_gaps_with_scope_boundaries_remain_reportable_but_material_unverified_blocks() {
        let (mut comparison, originals) = compared_fixture();
        comparison.inventory.request.original_request = "community\nunknown".into();
        comparison.inventory.scope_excluded.push(CareerRoute {
            owner: DataOwner::Company {
                company: "outside".into(),
            },
            kind: CareerRouteKind::Index,
            path: "vault/work/outside/experience/index.md".into(),
            label: "outside the approved scope".into(),
            supporting_sources: vec![],
        });
        comparison.inventory.inventory_digest = comparison.inventory.content_digest().unwrap();
        comparison.requirements[0].original_span.end = 9;
        comparison.requirements.push(CareerRequirement {
            id: "unknown".into(),
            original_span: CareerSpan { start: 10, end: 17 },
            kind: CareerRequirementKind::Requirement,
            coverage: CareerCoverage::Gap,
            material: true,
        });
        let gaps = comparison
            .decisions
            .iter()
            .cloned()
            .map(|mut d| {
                d.requirement_id = "unknown".into();
                d.disposition = CareerDisposition::Excluded;
                d.reason = "No supporting facet for this requirement in the approved scope".into();
                d
            })
            .collect::<Vec<_>>();
        comparison.decisions.extend(gaps);
        comparison
            .validate_current(&comparison.inventory, &originals, true)
            .unwrap();
        comparison.requirements[1].coverage = CareerCoverage::Unverified;
        assert!(comparison.validate_structure(true).is_err());
        comparison.requirements[1].material = false;
        assert!(
            comparison.validate_structure(true).is_ok(),
            "explicit nonmaterial uncertainty is a review-visible residual risk"
        );
    }
}
