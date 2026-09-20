//! Owned synthetic routing and byte-binding inputs, independent of private originals.
//! This module intentionally depends only on std and tempfile; callers own database work.
use std::{fs, path::Path};

pub const SCOPES: [&str; 5] = [
    "personal",
    "profile",
    "work/cluml",
    "work/common",
    "work/tmaxcloud",
];
pub const DOCUMENT_COUNT: usize = 55;
pub const TOTAL_BYTES: u64 = 11346;
// SHA-256 of the declared sorted path<TAB>byte length<TAB>content SHA-256<LF> records.
// Deliberate fixture contract changes must update this constant and the byte count.
pub const CONTRACT_DIGEST: &str =
    "d3c85f869a98241ee057a474bb089db37a225b894485e6f8bbbbfdf0d129b49e";

const PATHS: [&str; DOCUMENT_COUNT] = [
    "personal/business/index.md",
    "personal/decisions/ai-collaboration-values.md",
    "personal/decisions/resume-pdf-final-plan.md",
    "personal/decisions/solo-mvp-idea-discovery.md",
    "personal/facts/index.md",
    "personal/index.md",
    "personal/journal/index.md",
    "personal/knowledge/index.md",
    "personal/knowledge/skills.md",
    "personal/learning/index.md",
    "personal/ontology/index.md",
    "personal/ontology/schema.md",
    "personal/profile.md",
    "personal/projects/coupler.md",
    "personal/projects/gluesql/index.md",
    "personal/projects/ideas/idea-discovery-registry.md",
    "personal/projects/ideas/solo-founder-validation-platform.md",
    "personal/projects/meenseek-ontology.md",
    "personal/writing/career-output-assembly.md",
    "personal/writing/career-technical-portfolio-strategy.md",
    "personal/writing/case-docs/index.md",
    "personal/writing/claim-token-output-system.md",
    "personal/writing/contribution-audit.md",
    "personal/writing/portfolio-casebook.md",
    "personal/writing/resume-case-view.md",
    "personal/writing/resume-source-map.md",
    "profile/index.md",
    "profile/preferences/agent-operating-preferences.md",
    "profile/preferences/context-scope-routing.md",
    "profile/preferences/context-vault-operating-model.md",
    "profile/preferences/external-workspace-routing.md",
    "profile/rules/agent-harness.md",
    "profile/rules/common-code-quality.md",
    "profile/rules/common-document-quality.md",
    "profile/rules/common-review-quality.md",
    "profile/rules/context-doc-stability-review.md",
    "profile/rules/mandatory-preflight.md",
    "work/cluml/experience/case-studies/ci-runner-availability-monitoring.md",
    "work/cluml/experience/case-studies/hog-detection-period-config-externalization.md",
    "work/cluml/index.md",
    "work/cluml/preferences/cluml-routing.md",
    "work/cluml/projects/giganto.md",
    "work/cluml/rules/aice-aimer-auth-data-flow.md",
    "work/cluml/rules/cert.md",
    "work/cluml/rules/rust-issue-hunting.md",
    "work/cluml/rules/security-architecture-principles.md",
    "work/common/preferences/work-agent-operating-preferences.md",
    "work/common/router/agent-guide.md",
    "work/common/router/company-registry.md",
    "work/common/rules/code-review.md",
    "work/common/rules/dependency.md",
    "work/common/rules/design-discussion-note.md",
    "work/common/rules/issue.md",
    "work/common/rules/rust-code-style.md",
    "work/tmaxcloud/index.md",
];

const COMPANY_REGISTRY: &str = r#"# Synthetic company registry

## Company Registry

| Company | Company index |
| --- | --- |
| ClumL | `vault/work/cluml/index.md` |
| TmaxCloud | `vault/work/tmaxcloud/index.md` |
"#;
const COMPANY_ROUTING: &str = r#"---
domain_routes:
  - signals: [auth, jwt, oidc, session, token, mTLS, step-ca, aice, aimer]
    actions: [code-write, code-review, document-write, document-review, investigation, design]
    target_kind: any
    policies: [vault/work/cluml/rules/aice-aimer-auth-data-flow.md]
  - signals: [secret, vault, tenant, encrypt, 암호화, audit, 감사 로그, 개인정보]
    actions: [code-write, code-review, document-write, document-review, investigation, design]
    target_kind: any
    policies: [vault/work/cluml/rules/security-architecture-principles.md]
  - signals: [cert, certificate, CN, pem]
    actions: [code-write, code-review, document-write, document-review, investigation, design]
    target_kind: any
    policies: [vault/work/cluml/rules/cert.md]
  - signals: [dependency, crate, package, 버전]
    actions: [code-write, code-review, document-write, document-review, investigation, design]
    target_kind: any
    policies: [vault/work/common/rules/dependency.md]
  - signals: [issue hunting, 이슈헌팅, 할만한 이슈, 개선점 찾아줘, bug candidate, bug fix candidate, refactor candidate, refactoring candidate, performance candidate, performance improvement candidate, stability candidate, stability improvement candidate]
    actions: [investigation]
    target_kind: rust
    policies: [vault/work/cluml/rules/rust-issue-hunting.md, vault/profile/rules/common-code-quality.md, vault/work/common/rules/code-review.md, vault/work/common/rules/rust-code-style.md, vault/work/common/rules/issue.md]
---
# Synthetic company routing
"#;

/// The expected documents are derived from this declared contract, never an inventory scan.
pub fn documents() -> Vec<(&'static str, String)> {
    PATHS
        .iter()
        .map(|&path| {
            let content = match path {
                "work/common/router/company-registry.md" => COMPANY_REGISTRY.to_owned(),
                "work/cluml/preferences/cluml-routing.md" => COMPANY_ROUTING.to_owned(),
                _ => {
                    let scope = path.split('/').next().expect("declared fixture scope");
                    format!(
                        "---\ntitle: Synthetic routing fixture\nscope: {scope}\n---\n# Synthetic routing fixture\n\nvault/{path}: skills knowledge ontology project journal fixture.\n"
                    )
                }
            };
            (path, content)
        })
        .collect()
}

pub fn build() -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("owned synthetic context fixture");
    for (path, content) in documents() {
        let target = directory.path().join(Path::new(path));
        fs::create_dir_all(target.parent().expect("declared fixture file has a parent"))
            .expect("owned synthetic fixture directories");
        fs::write(target, content).expect("authored synthetic fixture bytes");
    }
    directory
}
