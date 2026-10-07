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
pub const TOTAL_BYTES: u64 = 4888;
// SHA256 of declared sorted path<TAB>length<TAB>content digest<LF> records.
pub const CONTRACT_DIGEST: &str =
    "2b80e493d4b1bf35d9af9bcad98964e5bfaf14b997ae52d1fb8c477a8ad7591b";
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
