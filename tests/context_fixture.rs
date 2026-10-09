//! Small fictional source/configuration fixture. It contains no installation bindings.
use context_core::harness::PolicyConfiguration;
use std::{
    fs,
    path::{Path, PathBuf},
};
pub const SCOPES: [&str; 5] = [
    "personal",
    "profile",
    "work/cedar",
    "work/common",
    "work/lumen",
];
pub const DOCUMENT_COUNT: usize = 32;
pub const TOTAL_BYTES: u64 = 4999;
// SHA256 of declared sorted path<TAB>length<TAB>content digest<LF> records.
pub const CONTRACT_DIGEST: &str =
    "68fdfd163402a8fb4060ab8ae6bb845e30b854e53077d9947fbd89c672e008d2";
const PATHS: [&str; DOCUMENT_COUNT] = [
    "personal/business/index.md",
    "personal/decisions/draft-note.md",
    "personal/facts/index.md",
    "personal/index.md",
    "personal/journal/index.md",
    "personal/knowledge/index.md",
    "personal/knowledge/skills.md",
    "personal/learning/index.md",
    "personal/ontology/index.md",
    "personal/ontology/schema.md",
    "personal/profile.md",
    "personal/projects/delta/index.md",
    "personal/projects/ideas/proposal.md",
    "personal/projects/ideas/tracker.md",
    "personal/projects/localexample.md",
    "personal/projects/sample.md",
    "personal/writing/evidence.md",
    "personal/writing/notes.md",
    "personal/writing/overview.md",
    "profile/index.md",
    "profile/preferences/sample-policy.md",
    "profile/rules/authoring.md",
    "profile/rules/checks.md",
    "profile/rules/control.md",
    "profile/rules/foundation.md",
    "profile/rules/language.md",
    "work/cedar/index.md",
    "work/cedar/preferences/routes.md",
    "work/cedar/projects/garden.md",
    "work/cedar/rules/widgets.md",
    "work/common/router/directory.md",
    "work/lumen/index.md",
];
const COMPANY_REGISTRY: &str = r#"## Company Registry

| Company | Source |
| --- | --- |
| Cedar Labs | `vault/work/cedar/index.md` |
| Lumen Group | `vault/work/lumen/index.md` |
"#;
const COMPANY_ROUTING: &str = r#"---
domain_routes:
  - signals: [widget]
    actions: [investigation]
    target_kind: any
    policies: [vault/work/cedar/rules/widgets.md]
---
# Fictional routing
"#;
pub fn configuration_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("crates/context-core/tests/fixtures/policy-settings.json")
}
pub fn configuration() -> PolicyConfiguration {
    PolicyConfiguration::read(configuration_path()).expect("fictional explicit policy input")
}
pub fn documents() -> Vec<(&'static str, String)> {
    PATHS.iter().map(|&path| {
        let content = match path {
            "work/common/router/directory.md" => COMPANY_REGISTRY.to_owned(),
            "work/cedar/preferences/routes.md" => COMPANY_ROUTING.to_owned(),
            "personal/writing/evidence.md" => "---\ntitle: Fictional routes\nscope: personal\n---\n# Routes\n```career-routes\n{\"routes\":[{\"owner\":{\"kind\":\"personal-project\",\"project\":\"sample\"},\"kind\":\"fact\",\"path\":\"vault/personal/projects/sample.md\",\"label\":\"sample work\"}]}\n```\n".into(),
            "personal/writing/overview.md" => "---\ntitle: Fictional auxiliaries\nscope: personal\n---\n# Sources\n```career-routes\n{\"routes\":[{\"owner\":{\"kind\":\"personal\"},\"kind\":\"fact\",\"path\":\"vault/personal/profile.md\",\"label\":\"profile\"}]}\n```\n".into(),
            _ => format!("---\ntitle: Fictional source\nscope: {}\n---\n# Fictional source\n\nvault/{path}: skills knowledge ontology project journal fixture.\n",path.split('/').next().expect("fixture scope")),
        };
        (path, content)
    }).collect()
}
pub fn build() -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("owned fixture");
    for (path, content) in documents() {
        let target = directory.path().join(path);
        fs::create_dir_all(target.parent().expect("fixture parent")).expect("fixture directories");
        fs::write(target, content).expect("fictional bytes");
    }
    directory
}

/// Builds transient comparison data from actual fixture source bytes and versions.
/// It never accepts a caller-invented inventory or creates production facts.
#[allow(
    dead_code,
    reason = "fixture is compiled separately by tests that do not all exercise career comparison"
)]
pub fn career_comparison(
    source: &dyn context_core::harness::ContextSource,
    original_request: &str,
    claim_id: &str,
    target: &str,
) -> context_core::career::CareerComparison {
    use context_core::{career::*, harness::*};
    use std::{collections::BTreeMap, io::Read};
    let mut originals = BTreeMap::new();
    let inventory = discover(
        &configuration(),
        source.store_identity().unwrap().map(|i| i.store_id),
        CareerDiscoveryRequest {
            original_request: original_request.into(),
            surface: CareerOutputSurface::Resume,
            approved_scopes: vec!["personal".into()],
            axis_queries: vec![],
        },
        |paths| {
            paths
                .iter()
                .map(|p| {
                    let mut content = String::new();
                    source
                        .open_file(Path::new(p), MAX_CAREER_SOURCE_BYTES as u64)?
                        .read_to_string(&mut content)
                        .unwrap();
                    let original = CareerOriginal {
                        path: p.clone(),
                        content,
                        version: source.metadata(Path::new(p))?.stored_version,
                    };
                    originals.insert(p.clone(), original.clone());
                    Ok(original)
                })
                .collect()
        },
    )
    .unwrap();
    let selected = "vault/personal/projects/sample.md";
    let comparison = CareerComparison {
        requirements: vec![CareerRequirement {
            id: "work".into(),
            original_span: CareerSpan {
                start: 0,
                end: original_request.len(),
            },
            kind: CareerRequirementKind::Requirement,
            coverage: CareerCoverage::Direct,
            material: true,
        }],
        decisions: inventory
            .candidates
            .iter()
            .map(|c| CareerDecision {
                requirement_id: "work".into(),
                candidate_path: c.path.clone(),
                disposition: if c.path == selected {
                    CareerDisposition::Selected
                } else {
                    CareerDisposition::Excluded
                },
                reason: if c.path == selected {
                    "source describes the fixture work"
                } else {
                    "source is a profile, not this fixture work"
                }
                .into(),
                facets: vec![CareerFacet {
                    axis: CareerAxis::Technical,
                    original: CareerLocator {
                        path: c.path.clone(),
                        original_span: CareerSpan { start: 0, end: 5 },
                    },
                }],
            })
            .collect(),
        placements: vec![CareerPlacement {
            claim_id: claim_id.into(),
            candidate_path: selected.into(),
            requirement_ids: vec!["work".into()],
            source_locators: vec![CareerLocator {
                path: selected.into(),
                original_span: CareerSpan { start: 0, end: 5 },
            }],
            target: target.into(),
            slot: "work".into(),
        }],
        inventory,
    };
    comparison
        .validate_current(&comparison.inventory, &originals, true)
        .unwrap();
    comparison
}
