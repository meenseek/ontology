use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    io::Read as _,
    path::{Component, Path, PathBuf},
};

use serde::Deserialize;
use syn::{
    Attribute, Expr, Field, ImplItemFn, ItemConst, ItemEnum, ItemFn, ItemImpl, ItemMod, ItemStruct,
    ItemType, ItemUnion, LitStr, Meta, TypePath,
    visit::{self, Visit},
};

use crate::{
    ContextVaultError, Result,
    frontmatter::ParsedFrontmatter,
    harness::{
        CompanyDomainRoute, CompanyDomainRouteTarget, HarnessAction, parse_company_domain_routes,
        parse_registered_company_slugs, verified_repository_file,
    },
};

const README_HARNESS_REQUIRED_UNITS: &[&str] = &[
    "vault/profile/rules/agent-harness.md",
    "docs/adr/0002-harness-single-current-contract.md",
    "one current schema",
    "Older records are rejected",
    "recoverable rather than filesystem-atomic",
    "separately verified derived-index receipt",
    "vault/profile/preferences/context-scope-routing.md",
];
const README_STALE_HARNESS_PHRASES: &[&str] = &[
    "Multi-file source mutation remains proposed",
    "the active apply path supports one file",
    "harness status --registry-root",
    "compatibility layer",
];
const README_DUPLICATED_COMPANY_ROUTING_PHRASE: &str = "ClumL company work";
const CURRENT_HARNESS_RUNTIME_FILES: &[&str] = &[
    "crates/context-core/src/harness.rs",
    "crates/context-core/src/harness/execution.rs",
    "crates/context-core/src/harness/finalization.rs",
    "crates/context-core/src/harness/persistence.rs",
    "crates/context-core/src/harness/repository.rs",
    "crates/context-core/src/harness/request.rs",
    "crates/context-core/src/harness/requirements.rs",
    "crates/context-core/src/harness/source.rs",
    "crates/context-core/src/harness/source_tests.rs",
    "crates/context-core/src/harness/tests.rs",
    "crates/context-core/src/harness/tool_plan.rs",
    "src/context_commit.rs",
    "src/lib.rs",
    "src/main.rs",
    "src/native_context.rs",
    "src/native_harness.rs",
    "src/native_role.rs",
];
const CURRENT_HARNESS_COMMANDS: &[&str] = &[
    "advance",
    "apply",
    "attest-career",
    "begin",
    "compose-career",
    "evaluate",
    "prepare",
    "recover",
    "replay",
    "resolve",
    "revise",
    "validate",
];
const REQUIRED_REPOSITORY_PATHS: &[&str] = &[
    "vault/profile/index.md",
    "vault/profile/rules/mandatory-preflight.md",
    "vault/personal/index.md",
    "vault/personal/business",
    "vault/personal/projects",
    "vault/personal/decisions",
    "vault/personal/facts/index.md",
    "vault/personal/knowledge",
    "vault/personal/journal/index.md",
    "vault/personal/learning",
    "vault/personal/ontology/index.md",
    "vault/personal/ontology/schema.md",
    "vault/work/common/router/agent-guide.md",
    "vault/work/cluml/index.md",
];
const ROUTING_GRAPH_ROOTS: &[&str] = &[
    "AGENTS.md",
    "vault/profile/index.md",
    "vault/profile/preferences/context-scope-routing.md",
    "vault/profile/rules/mandatory-preflight.md",
    "vault/work/common/router/agent-guide.md",
    "vault/work/common/router/company-registry.md",
];
const CLUML_ROUTING_PATH: &str = "vault/work/cluml/preferences/cluml-routing.md";
const CLUML_DOMAIN_ROUTING_CONTRACT: &[(&str, &str)] = &[
    (
        "auth",
        "vault/work/cluml/rules/aice-aimer-auth-data-flow.md",
    ),
    ("jwt", "vault/work/cluml/rules/aice-aimer-auth-data-flow.md"),
    (
        "oidc",
        "vault/work/cluml/rules/aice-aimer-auth-data-flow.md",
    ),
    (
        "session",
        "vault/work/cluml/rules/aice-aimer-auth-data-flow.md",
    ),
    (
        "token",
        "vault/work/cluml/rules/aice-aimer-auth-data-flow.md",
    ),
    (
        "mTLS",
        "vault/work/cluml/rules/aice-aimer-auth-data-flow.md",
    ),
    (
        "step-ca",
        "vault/work/cluml/rules/aice-aimer-auth-data-flow.md",
    ),
    (
        "aice",
        "vault/work/cluml/rules/aice-aimer-auth-data-flow.md",
    ),
    (
        "aimer",
        "vault/work/cluml/rules/aice-aimer-auth-data-flow.md",
    ),
    (
        "secret",
        "vault/work/cluml/rules/security-architecture-principles.md",
    ),
    (
        "vault",
        "vault/work/cluml/rules/security-architecture-principles.md",
    ),
    (
        "tenant",
        "vault/work/cluml/rules/security-architecture-principles.md",
    ),
    (
        "encrypt",
        "vault/work/cluml/rules/security-architecture-principles.md",
    ),
    (
        "암호화",
        "vault/work/cluml/rules/security-architecture-principles.md",
    ),
    (
        "audit",
        "vault/work/cluml/rules/security-architecture-principles.md",
    ),
    (
        "감사 로그",
        "vault/work/cluml/rules/security-architecture-principles.md",
    ),
    (
        "개인정보",
        "vault/work/cluml/rules/security-architecture-principles.md",
    ),
    ("cert", "vault/work/cluml/rules/cert.md"),
    ("certificate", "vault/work/cluml/rules/cert.md"),
    ("CN", "vault/work/cluml/rules/cert.md"),
    ("pem", "vault/work/cluml/rules/cert.md"),
    ("dependency", "vault/work/common/rules/dependency.md"),
    ("crate", "vault/work/common/rules/dependency.md"),
    ("package", "vault/work/common/rules/dependency.md"),
    ("버전", "vault/work/common/rules/dependency.md"),
];
const ENGINEERING_DOMAIN_ROUTE_ACTIONS: &[&str] = &[
    "code-write",
    "code-review",
    "document-write",
    "document-review",
    "investigation",
    "design",
];
const CLUML_ISSUE_HUNTING_SIGNALS: &[&str] = &[
    "issue hunting",
    "이슈헌팅",
    "할만한 이슈",
    "개선점 찾아줘",
    "bug candidate",
    "bug fix candidate",
    "refactor candidate",
    "refactoring candidate",
    "performance candidate",
    "performance improvement candidate",
    "stability candidate",
    "stability improvement candidate",
];
const CLUML_ISSUE_HUNTING_POLICIES: &[&str] = &[
    "vault/work/cluml/rules/rust-issue-hunting.md",
    "vault/profile/rules/common-code-quality.md",
    "vault/work/common/rules/code-review.md",
    "vault/work/common/rules/rust-code-style.md",
    "vault/work/common/rules/issue.md",
];

const DERIVED_PREFIXES: &[&str] = &[
    "vault/index/",
    "vault/indexes/",
    "vault/embeddings/",
    "vault/vector-store/",
    "vault/.cache/",
    "vault/cache/",
    "vault/tmp/",
    "vault/exports/",
];

const PERSONAL_ALLOWED_SEGMENTS: &[&str] = &[
    "business",
    "projects",
    "decisions",
    "facts",
    "knowledge",
    "journal",
    "learning",
    "ontology",
    "writing",
];
const WORK_COMMON_ALLOWED_SEGMENTS: &[&str] = &["preferences", "router", "rules"];
const WORK_COMPANY_ALLOWED_SEGMENTS: &[&str] = &[
    "preferences",
    "projects",
    "rules",
    "overview",
    "experience",
    "notion",
    "adapters",
    "ideas",
    "knowledge",
    "facts",
    "decisions",
];

const DESKTOP_AUDIT_PATH: &str = "docs/migration/20260612-desktop-docs-audit.md";
const DESKTOP_MANIFEST_PATH: &str = "docs/migration/20260612-desktop-docs-manifest.json";
const DEFAULT_RETIRED_ROOT: &str = "/Users/john/Desktop/docs.retired-2026612";

const PAGE_GRAPH_PLACEMENT_PATH: &str = "docs/migration/20260624-page-graph-placement-map.json";
const RETIRED_RAW_ROOT: &str = "vault/personal/notion-original-export";
const EXPECTED_PAGE_GRAPH_TOTAL: usize = 39;

#[derive(Debug, Default)]
struct Failures {
    messages: Vec<String>,
}

impl Failures {
    fn push(&mut self, message: impl Into<String>) {
        self.messages.push(message.into());
    }

    fn finish(self, heading: &str) -> Result<()> {
        if self.messages.is_empty() {
            return Ok(());
        }

        let mut message = String::from(heading);
        for failure in self.messages {
            message.push_str("\n- ");
            message.push_str(&failure);
        }
        Err(ContextVaultError::invalid_input(message))
    }
}

pub fn verify_vault_structure(repo_root: impl AsRef<Path>) -> Result<()> {
    let repo_root = repo_root.as_ref();
    let mut failures = Failures::default();

    assert_required_paths_exist(repo_root, &mut failures);
    let active_files = active_vault_files(repo_root, &mut failures)?;
    let active_markdown_files = active_files
        .iter()
        .filter(|relative_path| is_markdown_path(relative_path))
        .cloned()
        .collect::<Vec<_>>();

    assert_active_files_live_in_known_scopes(&active_files, &mut failures);
    assert_active_files_follow_canonical_layout(&active_files, &mut failures);
    assert_routing_document_graph_paths_exist(repo_root, &mut failures);
    assert_company_domain_routing_contracts(repo_root, &mut failures);
    assert_document_bundle_attachments(repo_root, &active_files, &mut failures);
    assert_frontmatter_scopes_match_paths(repo_root, &active_markdown_files, &mut failures);
    assert_learning_activity_dates(repo_root, &active_markdown_files, &mut failures);
    assert_llm_tools_are_adapters(repo_root, &active_markdown_files, &mut failures);
    assert_canonical_docs_keep_scope_boundaries(repo_root, &mut failures);
    assert_harness_adapter_contract(repo_root, &mut failures);
    assert_single_current_harness_architecture(repo_root, &mut failures);
    assert_derived_directories_are_ignored(repo_root, &mut failures);
    assert_no_source_markdown_in_raw_areas(repo_root, &mut failures)?;

    failures.finish("Vault structure verification failed:")
}

pub fn verify_migration(repo_root: impl AsRef<Path>) -> Result<()> {
    let repo_root = repo_root.as_ref();
    let audit_path = repo_root.join(DESKTOP_AUDIT_PATH);
    let manifest_path = repo_root.join(DESKTOP_MANIFEST_PATH);
    let retired_root = PathBuf::from(
        env::var("RETIRED_DESKTOP_DOCS").unwrap_or_else(|_| DEFAULT_RETIRED_ROOT.to_owned()),
    );
    let mut failures = Failures::default();

    if !audit_path.exists() {
        failures.push("Migration audit document is missing.");
    }
    if !manifest_path.exists() {
        failures.push("Migration manifest document is missing.");
    }
    if !failures.messages.is_empty() {
        return failures.finish("Migration verification failed:");
    }

    let manifest = read_json::<DesktopManifest>(&manifest_path)?;
    assert_desktop_manifest_shape(&manifest, &mut failures);
    assert_desktop_audit_covers_manifest(repo_root, &manifest, &mut failures)?;
    assert_desktop_repo_targets_exist(repo_root, &mut failures)?;
    assert_no_stale_operational_references(repo_root, &mut failures)?;
    verify_retired_root_if_available(repo_root, &retired_root, &manifest, &mut failures)?;

    failures.finish("Migration verification failed:")
}

pub fn verify_page_graph_inventory(repo_root: impl AsRef<Path>) -> Result<()> {
    let repo_root = repo_root.as_ref();
    let mut failures = Failures::default();

    if !repo_root.join(PAGE_GRAPH_PLACEMENT_PATH).exists() {
        failures.push(format!(
            "Missing page graph placement map: {PAGE_GRAPH_PLACEMENT_PATH}"
        ));
        return failures.finish("Personal page graph verification failed:");
    }

    let placement = read_json::<PageGraphPlacement>(&repo_root.join(PAGE_GRAPH_PLACEMENT_PATH))?;
    assert_page_graph_placement(repo_root, &placement, &mut failures)?;
    assert_retired_raw_tree_removed(repo_root, &mut failures);
    assert_retired_curated_outputs_removed(repo_root, &mut failures);

    failures.finish("Personal page graph verification failed:")
}

fn assert_required_paths_exist(repo_root: &Path, failures: &mut Failures) {
    for relative_path in REQUIRED_REPOSITORY_PATHS {
        if !repo_root.join(relative_path).exists() {
            failures.push(format!("Missing required repository path: {relative_path}"));
        }
    }
}

fn assert_company_domain_routing_contracts(repo_root: &Path, failures: &mut Failures) {
    let companies = match registered_company_slugs(repo_root) {
        Ok(companies) => companies,
        Err(error) => {
            failures.push(format!("Company registry cannot be safely parsed: {error}"));
            return;
        }
    };
    if !companies.contains("cluml") {
        failures.push("Company registry must contain the required `cluml` company index");
    }

    for company in companies {
        let routing_path = if company == "cluml" {
            CLUML_ROUTING_PATH.to_owned()
        } else {
            format!("vault/work/{company}/preferences/{company}-routing.md")
        };
        match fs::symlink_metadata(repo_root.join(&routing_path)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if company == "cluml" {
                    failures.push(format!(
                        "Missing required ClumL routing document: {routing_path}"
                    ));
                }
                continue;
            }
            Err(error) => {
                failures.push(format!(
                    "Company routing document metadata cannot be read: {routing_path}: {error}"
                ));
                continue;
            }
            Ok(_) => {}
        }

        let text = match read_verified_text(repo_root, &routing_path) {
            Ok(text) => text,
            Err(error) => {
                failures.push(format!(
                    "Company routing document cannot be safely read: {routing_path}: {error}"
                ));
                continue;
            }
        };
        let routes = match parse_company_domain_routes(&text, &company) {
            Ok(routes) => routes,
            Err(error) => {
                failures.push(format!(
                    "Invalid company domain routing contract `{routing_path}`: {error}"
                ));
                continue;
            }
        };

        for route in &routes {
            for policy in &route.policies {
                if let Err(error) = verified_repository_file(repo_root, Path::new(policy)) {
                    failures.push(format!(
                        "Company domain routing target cannot be safely opened: {routing_path} -> {policy}: {error}"
                    ));
                }
            }
        }
        if company == "cluml" {
            assert_cluml_domain_routing_contract(&routes, failures);
        }
    }
}

fn registered_company_slugs(repo_root: &Path) -> Result<BTreeSet<String>> {
    let registry_path = "vault/work/common/router/company-registry.md";
    let text = read_verified_text(repo_root, registry_path)?;
    parse_registered_company_slugs(&text).map_err(|error| {
        ContextVaultError::invalid_input(format!("invalid company registry: {error}"))
    })
}

fn assert_cluml_domain_routing_contract(routes: &[CompanyDomainRoute], failures: &mut Failures) {
    let actual = routes
        .iter()
        .flat_map(|route| {
            route.signals.iter().map(|signal| {
                (
                    signal.to_lowercase(),
                    route.policies.iter().cloned().collect::<BTreeSet<_>>(),
                    route
                        .actions
                        .iter()
                        .map(|action| action.as_str().to_owned())
                        .collect::<BTreeSet<_>>(),
                    route.target_kind,
                )
            })
        })
        .map(|(signal, policies, actions, target_kind)| (signal, (policies, actions, target_kind)))
        .collect::<BTreeMap<_, _>>();
    let mut expected_policies = BTreeMap::<String, BTreeSet<String>>::new();
    for (signal, policy) in CLUML_DOMAIN_ROUTING_CONTRACT {
        expected_policies
            .entry(signal.to_lowercase())
            .or_default()
            .insert((*policy).to_owned());
    }
    let engineering_actions = ENGINEERING_DOMAIN_ROUTE_ACTIONS
        .iter()
        .map(|action| (*action).to_owned())
        .collect::<BTreeSet<_>>();
    for (signal, policies) in expected_policies {
        match actual.get(&signal) {
            Some((actual_policies, actual_actions, actual_target_kind))
                if *actual_policies == policies
                    && *actual_actions == engineering_actions
                    && *actual_target_kind == CompanyDomainRouteTarget::Any => {}
            Some((actual_policies, actual_actions, actual_target_kind)) => failures.push(format!(
                "ClumL domain routing signal `{signal}` must have exactly policies [{}], actions [{}], and target kind `any`; got policies [{}], actions [{}], and target kind `{}`",
                policies.into_iter().collect::<Vec<_>>().join(", "),
                engineering_actions.iter().cloned().collect::<Vec<_>>().join(", "),
                actual_policies.iter().cloned().collect::<Vec<_>>().join(", "),
                actual_actions.iter().cloned().collect::<Vec<_>>().join(", "),
                actual_target_kind.as_str()
            )),
            None => failures.push(format!(
                "ClumL domain routing signal `{signal}` is missing"
            )),
        }
    }

    let issue_policies = CLUML_ISSUE_HUNTING_POLICIES
        .iter()
        .map(|policy| (*policy).to_owned())
        .collect::<BTreeSet<_>>();
    let investigation_action = [HarnessAction::Investigation.as_str().to_owned()]
        .into_iter()
        .collect::<BTreeSet<_>>();
    for signal in CLUML_ISSUE_HUNTING_SIGNALS {
        match actual.get(&signal.to_lowercase()) {
            Some((actual_policies, actual_actions, actual_target_kind))
                if *actual_policies == issue_policies
                    && *actual_actions == investigation_action
                    && *actual_target_kind == CompanyDomainRouteTarget::Rust => {}
            Some((actual_policies, actual_actions, actual_target_kind)) => failures.push(format!(
                "ClumL issue-hunting signal `{signal}` must have exactly the canonical policy bundle, `investigation` action, and `rust` target kind; got policies [{}], actions [{}], and target kind `{}`",
                actual_policies.iter().cloned().collect::<Vec<_>>().join(", "),
                actual_actions.iter().cloned().collect::<Vec<_>>().join(", "),
                actual_target_kind.as_str()
            )),
            None => failures.push(format!(
                "ClumL issue-hunting signal `{signal}` is missing"
            )),
        }
    }
}

fn assert_routing_document_graph_paths_exist(repo_root: &Path, failures: &mut Failures) {
    assert_routing_document_graph_paths_exist_from_roots(repo_root, ROUTING_GRAPH_ROOTS, failures);
}

fn assert_routing_document_graph_paths_exist_from_roots(
    repo_root: &Path,
    roots: &[&str],
    failures: &mut Failures,
) {
    let mut pending = roots.iter().map(ToString::to_string).collect::<Vec<_>>();
    let mut visited = BTreeSet::new();

    while let Some(routing_document) = pending.pop() {
        if !visited.insert(routing_document.clone()) {
            continue;
        }
        let text = match read_verified_text(repo_root, &routing_document) {
            Ok(text) => text,
            Err(error) => {
                failures.push(format!(
                    "Routing document cannot be safely read: {routing_document}: {error}"
                ));
                continue;
            }
        };

        let references = routing_markdown_references(&text);

        for (kind, reference) in references {
            match resolve_routing_document_reference(repo_root, &routing_document, &reference, kind)
            {
                Ok(Some(target)) => {
                    if is_routing_policy_document(&target) {
                        pending.push(target);
                    }
                }
                Ok(None) => {}
                Err(message) => failures.push(message),
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum RoutingReferenceKind {
    CodeSpan,
    MarkdownLink,
}

fn routing_markdown_references(text: &str) -> BTreeSet<(RoutingReferenceKind, String)> {
    let mut references = BTreeSet::new();
    let mut fence = None;

    for line in text.lines() {
        if let Some((marker, minimum_length)) = fence {
            if is_closing_fence(line, marker, minimum_length) {
                fence = None;
            }
            continue;
        }
        if let Some(marker) = opening_fence_marker(line) {
            fence = Some(marker);
            continue;
        }
        routing_inline_references(line, &mut references);
    }

    references
}

fn opening_fence_marker(line: &str) -> Option<(u8, usize)> {
    let indentation = line
        .as_bytes()
        .iter()
        .take_while(|byte| **byte == b' ')
        .count();
    if indentation > 3 {
        return None;
    }
    let trimmed = &line[indentation..];
    let marker = *trimmed.as_bytes().first()?;
    if !matches!(marker, b'`' | b'~') {
        return None;
    }
    let length = trimmed
        .as_bytes()
        .iter()
        .take_while(|candidate| **candidate == marker)
        .count();
    if length < 3 || (marker == b'`' && trimmed[length..].contains('`')) {
        return None;
    }
    Some((marker, length))
}

fn is_closing_fence(line: &str, marker: u8, minimum_length: usize) -> bool {
    let indentation = line
        .as_bytes()
        .iter()
        .take_while(|byte| **byte == b' ')
        .count();
    if indentation > 3 {
        return false;
    }
    let trimmed = &line[indentation..];
    let length = trimmed
        .as_bytes()
        .iter()
        .take_while(|candidate| **candidate == marker)
        .count();
    length >= minimum_length && trimmed[length..].trim().is_empty()
}

fn routing_inline_references(
    line: &str,
    references: &mut BTreeSet<(RoutingReferenceKind, String)>,
) {
    let bytes = line.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += 2;
            continue;
        }
        if bytes[index] == b'`' {
            let delimiter_length = byte_run_length(bytes, index, b'`');
            let content_start = index + delimiter_length;
            if let Some(content_end) = find_byte_run(bytes, content_start, b'`', delimiter_length) {
                references.insert((
                    RoutingReferenceKind::CodeSpan,
                    line[content_start..content_end].trim().to_owned(),
                ));
                index = content_end + delimiter_length;
                continue;
            }
        }
        if bytes[index] == b'['
            && let Some((destination, next_index)) = inline_link_destination(line, index)
        {
            references.insert((RoutingReferenceKind::MarkdownLink, destination));
            index = next_index;
            continue;
        }
        index += 1;
    }
}

fn byte_run_length(bytes: &[u8], start: usize, marker: u8) -> usize {
    bytes[start..]
        .iter()
        .take_while(|candidate| **candidate == marker)
        .count()
}

fn find_byte_run(bytes: &[u8], mut index: usize, marker: u8, length: usize) -> Option<usize> {
    while index < bytes.len() {
        if bytes[index] == marker {
            let candidate_length = byte_run_length(bytes, index, marker);
            if candidate_length == length {
                return Some(index);
            }
            index += candidate_length;
        } else {
            index += 1;
        }
    }
    None
}

fn inline_link_destination(line: &str, label_start: usize) -> Option<(String, usize)> {
    let bytes = line.as_bytes();
    let mut index = label_start + 1;
    let mut label_depth = 1;
    while index < bytes.len() && label_depth > 0 {
        match bytes[index] {
            b'\\' => index += 2,
            b'[' => {
                label_depth += 1;
                index += 1;
            }
            b']' => {
                label_depth -= 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
    if label_depth != 0 || bytes.get(index) != Some(&b'(') {
        return None;
    }

    index += 1;
    while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
        index += 1;
    }
    if bytes.get(index) == Some(&b'<') {
        let start = index + 1;
        let end = bytes[start..].iter().position(|byte| *byte == b'>')? + start;
        let close = link_closing_parenthesis(bytes, end + 1)?;
        return Some((line[start..end].to_owned(), close + 1));
    }

    let start = index;
    let mut nested_parentheses = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'(' => {
                nested_parentheses += 1;
                index += 1;
            }
            b')' if nested_parentheses == 0 => {
                return Some((line[start..index].to_owned(), index + 1));
            }
            b')' => {
                nested_parentheses -= 1;
                index += 1;
            }
            byte if byte.is_ascii_whitespace() && nested_parentheses == 0 => {
                let destination = line[start..index].to_owned();
                let close = link_closing_parenthesis(bytes, index)?;
                return Some((destination, close + 1));
            }
            _ => index += 1,
        }
    }
    None
}

fn link_closing_parenthesis(bytes: &[u8], mut index: usize) -> Option<usize> {
    while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
        index += 1;
    }
    let &title_delimiter = bytes.get(index)?;
    if title_delimiter == b')' {
        return Some(index);
    }
    if !matches!(title_delimiter, b'"' | b'\'' | b'(') {
        return None;
    }

    let title_end = if title_delimiter == b'(' {
        b')'
    } else {
        title_delimiter
    };
    index += 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            candidate if candidate == title_end => {
                index += 1;
                break;
            }
            _ => index += 1,
        }
    }
    while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
        index += 1;
    }
    (bytes.get(index) == Some(&b')')).then_some(index)
}

fn resolve_routing_document_reference(
    repo_root: &Path,
    routing_document: &str,
    reference: &str,
    kind: RoutingReferenceKind,
) -> std::result::Result<Option<String>, String> {
    let path = markdown_link_path(reference).trim();
    if path.is_empty()
        || is_external_link(path)
        || path.starts_with(['/', '#'])
        || path.contains(['<', '>', '{', '}', '*'])
        || path.contains(char::is_whitespace)
        || !is_markdown_path(path)
        || (kind == RoutingReferenceKind::CodeSpan
            && !path.contains('/')
            && !matches!(path, "AGENTS.md" | "README.md"))
    {
        return Ok(None);
    }
    if ["profile/", "personal/", "work/"]
        .iter()
        .any(|prefix| path.starts_with(prefix))
    {
        return Err(format!(
            "Routing document path must be repository-relative: {routing_document} -> {reference}"
        ));
    }

    let mut resolved = routing_reference_candidates(routing_document, path, kind)
        .into_iter()
        .filter_map(|candidate| canonical_repository_path(repo_root, &candidate))
        .collect::<BTreeSet<_>>();
    if resolved.is_empty() {
        return Err(format!(
            "Routing document path does not exist: {routing_document} -> {reference}"
        ));
    }
    if resolved.len() > 1 {
        return Err(format!(
            "Routing document path is ambiguous: {routing_document} -> {reference}: {}",
            resolved.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    Ok(resolved.pop_first())
}

fn routing_reference_candidates(
    routing_document: &str,
    reference: &str,
    kind: RoutingReferenceKind,
) -> Vec<PathBuf> {
    let source = Path::new(routing_document);
    if kind == RoutingReferenceKind::MarkdownLink {
        return vec![
            source
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .join(reference),
        ];
    }
    if reference.starts_with("vault/") || matches!(reference, "AGENTS.md" | "README.md") {
        return vec![PathBuf::from(reference)];
    }

    let mut bases = vec![
        source
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_path_buf(),
    ];
    if routing_document.starts_with("vault/profile/") {
        bases.push(PathBuf::from("vault/profile"));
    }
    if routing_document.starts_with("vault/work/") {
        bases.extend([
            PathBuf::from("vault/work"),
            PathBuf::from("vault/work/common"),
            PathBuf::from("vault/work/common/rules"),
        ]);
        if let Some(company) = source.components().nth(2) {
            let company_root = Path::new("vault/work").join(company.as_os_str());
            if company_root != Path::new("vault/work/common") {
                bases.push(company_root);
            }
        }
    }
    bases.push(PathBuf::new());

    bases
        .into_iter()
        .map(|base| base.join(reference))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn canonical_repository_path(repo_root: &Path, candidate: &Path) -> Option<String> {
    let relative = normalize_repository_relative_path(candidate)?;
    verified_repository_file(repo_root, &relative).ok()?;
    Some(to_posix(&relative))
}

fn normalize_repository_relative_path(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            Component::Prefix(_) | Component::RootDir => return None,
        }
    }
    (!normalized.as_os_str().is_empty()).then_some(normalized)
}

fn is_routing_policy_document(relative_path: &str) -> bool {
    relative_path == "AGENTS.md"
        || ((relative_path.starts_with("vault/profile/")
            || relative_path.starts_with("vault/work/"))
            && (relative_path.ends_with("/index.md")
                || ["/preferences/", "/router/", "/rules/"]
                    .iter()
                    .any(|segment| relative_path.contains(segment))))
}

fn active_vault_files(repo_root: &Path, failures: &mut Failures) -> Result<Vec<String>> {
    let vault_root = repo_root.join("vault");
    if !vault_root.exists() {
        failures.push("Missing vault root.");
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    for path in list_files(&vault_root)? {
        let relative_path = relative_to_root(repo_root, &path);
        if is_macos_metadata_path(&relative_path)
            || has_any_prefix(&relative_path, DERIVED_PREFIXES)
            || is_empty_context_save_lock(&path)?
        {
            continue;
        }
        files.push(relative_path);
    }
    files.sort();
    Ok(files)
}

fn is_empty_context_save_lock(path: &Path) -> Result<bool> {
    let Some(name) = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix('.'))
        .and_then(|name| name.strip_suffix(".md.save.lock"))
    else {
        return Ok(false);
    };
    if name.is_empty() {
        return Ok(false);
    }

    // Save and delete both leave a reusable sidecar, even when the Markdown is gone.
    let metadata = fs::symlink_metadata(path)
        .map_err(|source| ContextVaultError::io("inspect context save lock", path, source))?;
    if !metadata.is_file() || metadata.len() != 0 {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;

        if metadata.nlink() != 1 {
            return Ok(false);
        }
    }
    Ok(true)
}

fn assert_active_files_live_in_known_scopes(files: &[String], failures: &mut Failures) {
    for relative_path in files {
        let parts = split_path(relative_path);
        if parts.first() != Some(&"vault") {
            failures.push(format!(
                "Active file must live under vault: {relative_path}"
            ));
            continue;
        }
        match parts.get(1).copied() {
            Some("profile" | "personal" | "work") => {}
            _ => failures.push(format!(
                "Active vault file must live under profile, personal, or work: {relative_path}"
            )),
        }
    }
}

fn assert_active_files_follow_canonical_layout(files: &[String], failures: &mut Failures) {
    for relative_path in files {
        let parts = split_path(relative_path);
        match parts.get(1).copied() {
            Some("profile") if is_allowed_profile_path(&parts) => {}
            Some("personal") if is_allowed_personal_path(&parts) => {}
            Some("work") if is_allowed_work_path(&parts) => {}
            Some("profile" | "personal" | "work") => failures.push(format!(
                "Active vault file is outside the canonical scope layout: {relative_path}"
            )),
            _ => {}
        }
    }
}

fn is_allowed_profile_path(parts: &[&str]) -> bool {
    matches!(parts, ["vault", "profile", "index.md"])
        || parts.len() >= 4
            && parts.first() == Some(&"vault")
            && parts.get(1) == Some(&"profile")
            && matches!(
                parts.get(2),
                Some(
                    &"preferences"
                        | &"rules"
                        | &"working-style"
                        | &"ideas"
                        | &"knowledge"
                        | &"facts"
                        | &"decisions"
                )
            )
}

fn is_allowed_personal_path(parts: &[&str]) -> bool {
    matches!(parts, ["vault", "personal", "index.md"])
        || matches!(parts, ["vault", "personal", "profile.md"])
        || parts.len() >= 4
            && parts.first() == Some(&"vault")
            && parts.get(1) == Some(&"personal")
            && parts
                .get(2)
                .is_some_and(|segment| PERSONAL_ALLOWED_SEGMENTS.contains(segment))
}

fn assert_page_graph_placement(
    repo_root: &Path,
    placement: &PageGraphPlacement,
    failures: &mut Failures,
) -> Result<()> {
    if placement.page_count != EXPECTED_PAGE_GRAPH_TOTAL {
        failures.push(format!(
            "Unexpected page graph count: expected {EXPECTED_PAGE_GRAPH_TOTAL}, got {}",
            placement.page_count
        ));
    }
    if placement.pages.len() != placement.page_count {
        failures.push(format!(
            "Page graph map count mismatch: page_count={} entries={}",
            placement.page_count,
            placement.pages.len()
        ));
    }

    let mut targets = BTreeSet::new();
    for (source, target) in &placement.pages {
        if !is_markdown_path(source) {
            failures.push(format!("Page graph source is not Markdown: {source}"));
        }
        if !target.starts_with("vault/personal/") && !target.starts_with("vault/work/") {
            failures.push(format!(
                "Page graph target is outside active scopes: {target}"
            ));
        }
        if !targets.insert(target.as_str()) {
            failures.push(format!("Page graph target is duplicated: {target}"));
        }

        let absolute_target = repo_root.join(target);
        if !absolute_target.exists() {
            failures.push(format!("Page graph target is missing: {target}"));
            continue;
        }
        if !is_markdown_path(target) {
            failures.push(format!("Page graph target must be Markdown: {target}"));
            continue;
        }

        let text = fs::read_to_string(&absolute_target)
            .map_err(|error| ContextVaultError::io("read file", &absolute_target, error))?;
        if let Some(expected_links) = placement.source_external_links.get(source) {
            for link in expected_links {
                if !text.contains(link) {
                    failures.push(format!(
                        "Page graph target is missing source external link: {target} -> {link}"
                    ));
                }
            }
        }
        for forbidden in [
            "notion_page_id",
            "raw_json",
            "## Source Coverage",
            "Migration bucket:",
            "curated 기준",
            "원본 Notion",
        ] {
            if text.contains(forbidden) {
                failures.push(format!(
                    "Page graph target keeps retired migration text: {target} -> {forbidden}"
                ));
            }
        }

        for link in markdown_links(&text) {
            let link_path = markdown_link_path(&link);
            if link.contains("prod-files-secure.s3") {
                failures.push(format!(
                    "Page graph target keeps Notion-hosted file link: {target} -> {link}"
                ));
            }

            if is_external_link(&link) || link.starts_with('/') || link.starts_with('#') {
                continue;
            }

            let resolved = absolute_target
                .parent()
                .unwrap_or(repo_root)
                .join(link_path);
            if link_path.contains(".md") && !resolved.exists() {
                failures.push(format!(
                    "Page graph target has broken Markdown link: {target} -> {link}"
                ));
            }
            if is_image_path(link_path) && !resolved.exists() {
                failures.push(format!(
                    "Page graph target has broken local image link: {target} -> {link}"
                ));
            }
        }
    }

    Ok(())
}

fn assert_retired_curated_outputs_removed(repo_root: &Path, failures: &mut Failures) {
    for relative_path in [
        "vault/personal/learning/boostcamp.md",
        "vault/personal/learning/samsung-sds-2021-summer-algorithm-camp.md",
        "vault/personal/writing/resume-profile.md",
        "vault/work/tmaxcloud/overview/experience/index.md",
        "vault/work/tmaxcloud/overview/experience/attachments",
    ] {
        if repo_root.join(relative_path).exists() {
            failures.push(format!(
                "Retired curated output should be removed: {relative_path}"
            ));
        }
    }
}

fn markdown_links(text: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("](") {
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find(')') else {
            break;
        };
        let link = &after_start[..end];
        links.push(link.to_owned());
        rest = &after_start[end + 1..];
    }
    links
}

fn markdown_link_path(link: &str) -> &str {
    let fragment_index = link.find('#').unwrap_or(link.len());
    let query_index = link.find('?').unwrap_or(link.len());
    &link[..fragment_index.min(query_index)]
}

fn is_external_link(link: &str) -> bool {
    link.contains("://") || link.starts_with("mailto:")
}

fn is_image_path(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    matches!(
        Path::new(&path)
            .extension()
            .and_then(|extension| extension.to_str()),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "svg")
    )
}

fn is_allowed_work_path(parts: &[&str]) -> bool {
    if matches!(parts, ["vault", "work", "common", category, ..] if WORK_COMMON_ALLOWED_SEGMENTS.contains(category))
    {
        return true;
    }

    matches!(parts, ["vault", "work", company, "index.md"] if *company != "common")
        || parts.len() >= 5
            && parts.first() == Some(&"vault")
            && parts.get(1) == Some(&"work")
            && parts.get(2) != Some(&"common")
            && parts
                .get(3)
                .is_some_and(|segment| WORK_COMPANY_ALLOWED_SEGMENTS.contains(segment))
}

fn assert_document_bundle_attachments(repo_root: &Path, files: &[String], failures: &mut Failures) {
    for relative_path in files {
        if is_original_import_path(relative_path) {
            continue;
        }

        let parts = split_path(relative_path);
        if is_markdown_path(relative_path) {
            if parts.contains(&"attachments") {
                failures.push(format!(
                    "Markdown must not live inside attachments: {relative_path}"
                ));
            }
            continue;
        }

        let Some(bundle_index) = document_bundle_index_path(relative_path) else {
            failures.push(format!(
                "Active non-Markdown file must live under a document bundle attachments directory: {relative_path}"
            ));
            continue;
        };

        let absolute_index = repo_root.join(&bundle_index);
        if !absolute_index.exists() {
            failures.push(format!(
                "Attachment bundle is missing index.md: {relative_path} -> {bundle_index}"
            ));
            continue;
        }

        let text = match fs::read_to_string(&absolute_index) {
            Ok(text) => text,
            Err(source) => {
                failures.push(format!(
                    "Failed to read attachment bundle index `{bundle_index}`: {source}"
                ));
                continue;
            }
        };

        let attachment_name = parts.last().copied().unwrap_or_default();
        let expected_reference = format!("attachments/{attachment_name}");
        if !text.contains(&expected_reference) {
            failures.push(format!(
                "Attachment is not referenced from its bundle index: {relative_path}"
            ));
        }
    }
}

fn is_original_import_path(relative_path: &str) -> bool {
    split_path(relative_path)
        .windows(2)
        .any(|segments| segments == ["notion", "originals"])
}

fn document_bundle_index_path(relative_path: &str) -> Option<String> {
    let parts = split_path(relative_path);
    let attachments_index = parts.iter().position(|segment| *segment == "attachments")?;
    if attachments_index < 3 || attachments_index + 2 != parts.len() {
        return None;
    }

    let bundle_parts = &parts[..attachments_index];
    Some(format!("{}/index.md", bundle_parts.join("/")))
}

fn assert_frontmatter_scopes_match_paths(
    repo_root: &Path,
    files: &[String],
    failures: &mut Failures,
) {
    for relative_path in files {
        let text = match read_text(repo_root, relative_path) {
            Ok(text) => text,
            Err(error) => {
                failures.push(error.to_string());
                continue;
            }
        };
        let Some(frontmatter) = verified_frontmatter(&text, relative_path, failures) else {
            continue;
        };
        let scope = match frontmatter.optional_scalar_text("scope") {
            Ok(Some(scope)) => scope,
            Ok(None) => continue,
            Err(error) => {
                failures.push(format!("Invalid frontmatter in {relative_path}: {error}"));
                continue;
            }
        };
        let path_scope = split_path(relative_path)
            .get(1)
            .copied()
            .unwrap_or_default();
        if scope != path_scope {
            failures.push(format!(
                "Frontmatter scope does not match path: {relative_path} has scope {scope}"
            ));
        }
    }
}

fn assert_learning_activity_dates(repo_root: &Path, files: &[String], failures: &mut Failures) {
    for relative_path in files {
        if !relative_path.starts_with("vault/personal/learning/") {
            continue;
        }

        let text = match read_text(repo_root, relative_path) {
            Ok(text) => text,
            Err(error) => {
                failures.push(error.to_string());
                continue;
            }
        };
        let Some(frontmatter) = verified_frontmatter(&text, relative_path, failures) else {
            continue;
        };
        let document_type = match frontmatter.optional_scalar_text("type") {
            Ok(value) => value,
            Err(error) => {
                failures.push(format!("Invalid frontmatter in {relative_path}: {error}"));
                continue;
            }
        };
        if document_type.as_deref() != Some("learning-activity") {
            continue;
        }

        for field in ["started_on", "ended_on"] {
            let value = frontmatter.optional_scalar_text(field).ok().flatten();
            if !value.is_some_and(|value| is_iso_date(&value)) {
                failures.push(format!(
                    "Learning activity must have YYYY-MM-DD {field}: {relative_path}"
                ));
            }
        }

        let Some(problem_count) = frontmatter
            .optional_scalar_text("problem_count")
            .ok()
            .flatten()
            .and_then(|value| value.parse::<usize>().ok())
        else {
            continue;
        };
        let source_block = fenced_markdown_block(&text).unwrap_or_default();
        let baekjoon_links = source_block
            .matches("https://www.acmicpc.net/problem/")
            .count();
        let unchecked_items = source_block.matches("[ ]").count();
        if baekjoon_links != problem_count {
            failures.push(format!(
                "Learning activity problem_count does not match Baekjoon links: {relative_path} ({problem_count} != {baekjoon_links})"
            ));
        }
        if unchecked_items != problem_count {
            failures.push(format!(
                "Learning activity problem_count does not match checklist items: {relative_path} ({problem_count} != {unchecked_items})"
            ));
        }
    }
}

fn assert_llm_tools_are_adapters(repo_root: &Path, files: &[String], failures: &mut Failures) {
    for relative_path in files {
        let parts = split_path(relative_path)
            .into_iter()
            .map(str::to_ascii_lowercase)
            .collect::<Vec<_>>();
        let has_tool_name = parts.iter().any(|part| part == "codex" || part == "claude");
        let is_adapter = parts.iter().any(|part| part == "adapters");

        if has_tool_name && !is_adapter {
            failures.push(format!(
                "LLM tool-specific path must be an adapter, not a scope: {relative_path}"
            ));
        }

        if !is_adapter {
            continue;
        }

        let valid_company_adapter = parts.first().is_some_and(|part| part == "vault")
            && parts.get(1).is_some_and(|part| part == "work")
            && parts.get(2).is_some_and(|part| part != "common")
            && parts.get(3).is_some_and(|part| part == "adapters");
        if !valid_company_adapter {
            failures.push(format!(
                "Adapter files must live under vault/work/<company>/adapters: {relative_path}"
            ));
        }

        let text = match read_text(repo_root, relative_path) {
            Ok(text) => text.to_ascii_lowercase(),
            Err(error) => {
                failures.push(error.to_string());
                continue;
            }
        };
        let points_to_source = text.contains("source") || text.contains("원문");
        if !text.contains("adapter") || !points_to_source {
            failures.push(format!(
                "Adapter file must identify itself as a pointer to source documents: {relative_path}"
            ));
        }
    }
}

fn assert_canonical_docs_keep_scope_boundaries(repo_root: &Path, failures: &mut Failures) {
    let docs = [
        ("AGENTS.md", "승격된 Markdown 검증이 끝나면 제거한다"),
        (
            "vault/profile/preferences/context-vault-operating-model.md",
            "승격이 끝난 임시 복사본은 남기지 않는다",
        ),
        (
            "README.md",
            "`personal` means user-owned non-company context",
        ),
        (
            "README.md",
            "Codex, Claude, and other LLM tools are runtime adapters",
        ),
        (
            "README.md",
            "Document bundles keep Markdown and attachments together",
        ),
        ("docs/scope-model.md", "LLM tool names are not scopes"),
        (
            "docs/scope-model.md",
            "Attachments stay inside the owning document bundle",
        ),
        (
            "docs/scope-model.md",
            "Migration raw sources are temporary inputs",
        ),
        (
            "vault/personal/writing/resume-source-map.md",
            "회사 경력이 포함된 작업에서는 `work` scope 문서를 읽는다",
        ),
        (
            "vault/personal/writing/resume-source-map.md",
            "개인 프로젝트와 오픈소스만 다루면 `personal/projects` source를 읽는다",
        ),
        (
            "vault/personal/index.md",
            "개인 소유의 사업, 제품, 지식, 학습, 글쓰기, 의사결정",
        ),
        (
            "vault/personal/ontology/index.md",
            "Markdown 기반 ontology projection",
        ),
        ("vault/personal/ontology/schema.md", "type"),
        ("vault/personal/ontology/schema.md", "entities"),
        ("vault/personal/ontology/schema.md", "relations"),
        ("vault/personal/ontology/schema.md", "applies_to"),
    ];

    for (relative_path, snippet) in docs {
        match read_text(repo_root, relative_path) {
            Ok(text) if text.contains(snippet) => {}
            Ok(_) => failures.push(format!(
                "Required scope or ontology statement is missing: {snippet}"
            )),
            Err(error) => failures.push(error.to_string()),
        }
    }
}

fn assert_harness_adapter_contract(repo_root: &Path, failures: &mut Failures) {
    let required_snippets = [
        ("AGENTS.md", "harness resolve --json"),
        ("AGENTS.md", "harness prepare"),
        ("AGENTS.md", "unsupported runtime"),
        ("AGENTS.md", "canonical 절차만 따른다"),
        ("AGENTS.md", "Harness 호환 예산은 0"),
        ("docs/harness-adapter-contract.md", "harness resolve"),
        ("docs/harness-adapter-contract.md", "harness prepare"),
        ("docs/harness-adapter-contract.md", "harness begin"),
        ("docs/harness-adapter-contract.md", "harness advance"),
        ("docs/harness-adapter-contract.md", "harness evaluate"),
        ("docs/harness-adapter-contract.md", "harness validate"),
        ("docs/harness-adapter-contract.md", "harness apply"),
        ("docs/harness-adapter-contract.md", "harness recover"),
        ("docs/harness-adapter-contract.md", "advisory"),
        ("docs/harness-adapter-contract.md", "RequestEnvelope"),
        ("docs/harness-adapter-contract.md", "HARNESS_SCHEMA_VERSION"),
        (
            "docs/harness-adapter-contract.md",
            "There is no long-lived admission",
        ),
        (
            "docs/harness-adapter-contract.md",
            "Task-specific canonical policy",
        ),
    ];

    for (relative_path, snippet) in required_snippets {
        match read_text(repo_root, relative_path) {
            Ok(text) if text.contains(snippet) => {}
            Ok(_) => failures.push(format!(
                "Required Harness adapter contract is missing from {relative_path}: {snippet}"
            )),
            Err(error) => failures.push(error.to_string()),
        }
    }
    let adapter_path = "docs/harness-adapter-contract.md";
    match read_text(repo_root, adapter_path) {
        Ok(text) => assert_professional_profile_adapter_pointer(&text, failures),
        Err(error) => failures.push(error.to_string()),
    }
    assert_readme_harness_contract(repo_root, failures);
    assert_harness_operating_contract(repo_root, failures);
}

fn assert_professional_profile_adapter_pointer(text: &str, failures: &mut Failures) {
    let Some(section) = markdown_level_two_section(text, "Professional profile boundary") else {
        failures.push(
            "Harness adapter contract is missing the Professional profile boundary section"
                .to_owned(),
        );
        return;
    };
    let canonical_boundary =
        "../vault/profile/rules/agent-harness.md#professional-profile-boundary";
    if !section.contains(canonical_boundary) {
        failures.push(format!(
            "Professional-profile adapter section must point to the canonical boundary: {canonical_boundary}"
        ));
    }
    for duplicated_policy in [
        "ProfessionalProfileArtifact",
        "LinkedIn",
        "Wanted",
        "Remember",
        "external publishing",
        "`Improvement` correction",
    ] {
        if section.contains(duplicated_policy) {
            failures.push(format!(
                "Harness adapter contract duplicates canonical professional-profile policy: {duplicated_policy}"
            ));
        }
    }
}

fn markdown_level_two_section<'a>(text: &'a str, heading: &str) -> Option<&'a str> {
    let marker = format!("## {heading}\n");
    let body = &text[text.find(&marker)? + marker.len()..];
    Some(&body[..body.find("\n## ").unwrap_or(body.len())])
}

fn assert_readme_harness_contract(repo_root: &Path, failures: &mut Failures) {
    let relative_path = "README.md";
    let text = match read_text(repo_root, relative_path) {
        Ok(text) => text,
        Err(error) => {
            failures.push(error.to_string());
            return;
        }
    };
    for &required_unit in README_HARNESS_REQUIRED_UNITS {
        if !text.contains(required_unit) {
            failures.push(format!(
                "README active Harness contract is missing required semantic unit: {required_unit}"
            ));
        }
    }
    for &stale_phrase in README_STALE_HARNESS_PHRASES {
        if text.contains(stale_phrase) {
            failures.push(format!(
                "README contains stale Harness contract phrase: {stale_phrase}"
            ));
        }
    }
    if text.contains(README_DUPLICATED_COMPANY_ROUTING_PHRASE) {
        failures.push(format!(
            "README duplicates company-specific routing instead of using the canonical scope pointer: {README_DUPLICATED_COMPANY_ROUTING_PHRASE}"
        ));
    }
}

fn assert_single_current_harness_architecture(repo_root: &Path, failures: &mut Failures) {
    assert_native_harness_architecture(repo_root, failures);

    for relative_path in [
        "AGENTS.md",
        "README.md",
        "docs/harness-adapter-contract.md",
        "docs/adr/0002-harness-single-current-contract.md",
        "vault/profile/rules/agent-harness.md",
        "vault/profile/preferences/agent-harness-operating-plan.md",
    ] {
        match read_text(repo_root, relative_path) {
            Ok(text) if contains_dash_v_digit(&text) => failures.push(format!(
                "Active Harness document contains a versioned command token: {relative_path}"
            )),
            Ok(_) => {}
            Err(error) => failures.push(error.to_string()),
        }
    }
}

// This gate inspects executable code only. Filesystem Vault/document migration checks remain
// in verify_vault_structure and never provide evidence for the native runtime architecture.
#[allow(
    clippy::too_many_lines,
    reason = "one inventory audit combines exact module, decoder and native dispatch ownership"
)]
fn assert_native_harness_architecture(repo_root: &Path, failures: &mut Failures) {
    assert_native_harness_manifests(repo_root, failures);
    let mut sources = BTreeMap::new();
    for &path in CURRENT_HARNESS_RUNTIME_FILES
        .iter()
        .chain(["crates/context-core/src/lib.rs"].iter())
    {
        match read_verified_text(repo_root, path).and_then(|text| {
            syn::parse_file(&text).map_err(|error| {
                ContextVaultError::invalid_input(format!("invalid Rust in {path}: {error}"))
            })
        }) {
            Ok(syntax) => {
                sources.insert(path, syntax);
            }
            Err(error) => failures.push(error.to_string()),
        }
    }
    let directory = repo_root.join("crates/context-core/src/harness");
    match fs::read_dir(&directory) {
        Ok(entries) => {
            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) => {
                        failures.push(format!("Cannot inspect Harness inventory: {error}"));
                        continue;
                    }
                };
                let path = relative_to_root(repo_root, &entry.path());
                if !CURRENT_HARNESS_RUNTIME_FILES.contains(&path.as_str()) {
                    failures.push(format!(
                        "Harness runtime source inventory is closed: {path}"
                    ));
                }
            }
        }
        Err(error) => failures.push(format!("Cannot inspect Harness inventory: {error}")),
    }
    let mut schemas = 0;
    let mut decoders = 0;
    let mut core_types = BTreeSet::new();
    for (&path, syntax) in &sources {
        if path.starts_with("crates/context-core/src/harness") && !path.ends_with("tests.rs") {
            let mut audit = HarnessSourceAudit::default();
            audit.visit_file(syntax);
            schemas += audit.version_constants;
            for message in audit.violations {
                failures.push(format!("{path}: {message}"));
            }
            for item in &syntax.items {
                match item {
                    syn::Item::Struct(item) if !has_cfg_test(&item.attrs) => {
                        core_types.insert(item.ident.to_string());
                    }
                    syn::Item::Enum(item) if !has_cfg_test(&item.attrs) => {
                        core_types.insert(item.ident.to_string());
                    }
                    syn::Item::Type(item) if !has_cfg_test(&item.attrs) => {
                        core_types.insert(item.ident.to_string());
                    }
                    syn::Item::Fn(item)
                        if item.sig.ident == "decode_current_json"
                            && !has_cfg_test(&item.attrs) =>
                    {
                        decoders += 1;
                        if path != "crates/context-core/src/harness.rs" {
                            failures
                                .push("Canonical Harness decoder must belong to Core harness.rs");
                        }
                        assert_canonical_decoder(item, failures);
                    }
                    _ => {}
                }
            }
        }
        assert_native_module_wiring(path, syntax, failures);
    }
    if schemas != 1 {
        failures.push(format!(
            "Harness runtime must declare exactly one HARNESS_SCHEMA_VERSION, found {schemas}"
        ));
    }
    if decoders != 1 {
        failures.push(format!(
            "Harness runtime must declare exactly one canonical decoder, found {decoders}"
        ));
    }
    for (&path, syntax) in &sources {
        if path.starts_with("src/") {
            let mut audit = NativeBoundaryAudit {
                path,
                core_types: &core_types,
                failures,
                function: String::new(),
                canonical_decoder_calls: 0,
                core_imports: 0,
                main_dispatches: 0,
            };
            audit.visit_file(syntax);
            if matches!(
                path,
                "src/native_harness.rs"
                    | "src/native_role.rs"
                    | "src/native_context.rs"
                    | "src/context_commit.rs"
            ) && audit.core_imports == 0
            {
                audit.failures.push(format!(
                    "{path} must import its Harness interfaces from context_core::harness"
                ));
            }
            if path == "src/main.rs" && audit.main_dispatches != 1 {
                audit.failures.push("Native main must dispatch exactly once to meenseek_ontology::native_harness::run");
            }
            if matches!(path, "src/native_harness.rs" | "src/native_role.rs")
                && audit.canonical_decoder_calls == 0
            {
                audit
                    .failures
                    .push(format!("{path} must use the canonical Core decoder"));
            }
            if path == "src/native_harness.rs" {
                let mut commands = HarnessSourceAudit {
                    dispatch_function: "run",
                    ..HarnessSourceAudit::default()
                };
                commands.visit_file(syntax);
                assert_current_command_set(&commands.harness_commands, failures);
                // Native command records use the same strict serde/type/decoder rules as Core.
                for message in commands.violations {
                    failures.push(format!("{path}: {message}"));
                }
            }
        }
    }
}

fn assert_native_harness_manifests(repo_root: &Path, failures: &mut Failures) {
    for (path, required) in [
        (
            "Cargo.toml",
            &[
                "name=\"meenseek-ontology\"",
                "members=[\"crates/context-core\"]",
                "context-core={path=\"crates/context-core\"}",
            ][..],
        ),
        (
            "crates/context-core/Cargo.toml",
            &["name=\"context-core\"", "autobins=false"][..],
        ),
    ] {
        let text = match read_verified_text(repo_root, path) {
            Ok(text) => text,
            Err(error) => {
                failures.push(error.to_string());
                continue;
            }
        };
        let lines = text
            .lines()
            .map(|line| {
                line.chars()
                    .filter(|character| !character.is_whitespace())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        for required in required {
            if lines
                .iter()
                .filter(|line| line.as_str() == *required)
                .count()
                != 1
            {
                failures.push(format!("{path} must bind exactly one {required}"));
            }
        }
        if lines.iter().any(|line| {
            line == "[features]" || line == "[[bin]]" || line.starts_with("default-run=")
        }) {
            failures.push(format!(
                "{path} must not add a parallel Harness feature or binary"
            ));
        }
    }
}

fn assert_native_module_wiring(path: &str, syntax: &syn::File, failures: &mut Failures) {
    let expected: &[&str] = match path {
        "crates/context-core/src/lib.rs" => &["harness"],
        "crates/context-core/src/harness.rs" => &[
            "execution",
            "finalization",
            "persistence",
            "repository",
            "request",
            "requirements",
            "source",
            "source_tests",
            "tests",
            "tool_plan",
        ],
        "src/lib.rs" => &[
            "context_commit",
            "native_context",
            "native_harness",
            "native_role",
        ],
        _ => &[],
    };
    let mut counts = BTreeMap::<String, usize>::new();
    for item in &syntax.items {
        let syn::Item::Mod(module) = item else {
            continue;
        };
        let name = module.ident.to_string();
        let runtime_path = module.attrs.iter().any(|attribute| {
            attribute.path().is_ident("path") && matches!(&attribute.meta,
                Meta::NameValue(value) if matches!(&value.value, Expr::Lit(literal)
                    if matches!(&literal.lit, syn::Lit::Str(path) if path.value().contains("harness") || path.value().contains("context-core"))))
        });
        let relevant = runtime_path
            || expected.contains(&name.as_str())
            || (path.starts_with("crates/context-core/src/harness")
                && !has_cfg_test(&module.attrs))
            || matches!(
                name.as_str(),
                "harness" | "native_harness" | "native_role" | "native_context" | "context_commit"
            );
        if !relevant {
            continue;
        }
        *counts.entry(name.clone()).or_default() += 1;
        if !expected.contains(&name.as_str()) || module.content.is_some() {
            failures.push(format!(
                "{path}: unexpected or duplicate implementation module {name}"
            ));
        }
        let attributes = module
            .attrs
            .iter()
            .filter(|attribute| attribute.path().is_ident("path"))
            .collect::<Vec<_>>();
        let test_module = path == "crates/context-core/src/harness.rs"
            && matches!(name.as_str(), "tests" | "source_tests");
        if test_module {
            let valid = attributes.len() == 1
                && has_cfg_test(&module.attrs)
                && matches!(&attributes[0].meta, Meta::NameValue(value) if matches!(&value.value, Expr::Lit(literal) if matches!(&literal.lit, syn::Lit::Str(value) if value.value() == format!("harness/{name}.rs"))));
            if !valid {
                failures.push(format!(
                    "{path}: test inventory module {name} must bind its exact owned file"
                ));
            }
        } else if !attributes.is_empty() || has_cfg_test(&module.attrs) {
            failures.push(format!(
                "{path}: production module {name} must use its canonical file"
            ));
        }
    }
    for name in expected {
        if counts.get(*name) != Some(&1) {
            failures.push(format!("{path}: require exactly one module {name}"));
        }
    }
}

#[derive(Default)]
struct DecoderShape {
    calls: BTreeSet<String>,
    exact_shape_comparison: bool,
    unique_key_check: bool,
}
impl<'ast> Visit<'ast> for DecoderShape {
    fn visit_expr_call(&mut self, expression: &'ast syn::ExprCall) {
        if let Expr::Path(path) = expression.func.as_ref() {
            self.calls.insert(
                path.path
                    .segments
                    .iter()
                    .map(|segment| segment.ident.to_string())
                    .collect::<Vec<_>>()
                    .join("::"),
            );
        }
        visit::visit_expr_call(self, expression);
    }
    fn visit_expr_binary(&mut self, expression: &'ast syn::ExprBinary) {
        if matches!(expression.op, syn::BinOp::Ne(_))
            && matches!(expression.left.as_ref(), Expr::Path(path) if path.path.is_ident("input"))
            && matches!(expression.right.as_ref(), Expr::Path(path) if path.path.is_ident("canonical"))
        {
            self.exact_shape_comparison = true;
        }
        visit::visit_expr_binary(self, expression);
    }
    fn visit_expr_method_call(&mut self, expression: &'ast syn::ExprMethodCall) {
        if expression.method == "insert"
            && matches!(expression.receiver.as_ref(), Expr::Path(path) if path.path.is_ident("unique_keys"))
        {
            self.unique_key_check = true;
        }
        visit::visit_expr_method_call(self, expression);
    }
}
fn assert_canonical_decoder(item: &ItemFn, failures: &mut Failures) {
    let mut shape = DecoderShape::default();
    shape.visit_item_fn(item);
    if !shape.exact_shape_comparison
        || !shape.unique_key_check
        || [
            "serde_json::from_slice",
            "serde_json::from_value",
            "serde_json::to_value",
            "JsonObjectKeyScanner::scan",
        ]
        .iter()
        .any(|call| !shape.calls.contains(*call))
    {
        failures.push("Canonical decoder must retain duplicate-key scanning and exact round-trip shape validation");
    }
}

struct NativeBoundaryAudit<'a> {
    path: &'a str,
    core_types: &'a BTreeSet<String>,
    failures: &'a mut Failures,
    function: String,
    canonical_decoder_calls: usize,
    core_imports: usize,
    main_dispatches: usize,
}
impl NativeBoundaryAudit<'_> {
    fn inspect_role_decoder(&mut self, signature: &syn::Signature, block: &syn::Block) {
        if self.path != "src/native_role.rs" {
            return;
        }
        let mut audit = NativeRoleDecoderAudit {
            core_types: self.core_types,
            has_core_type: false,
            decoder: HarnessSourceAudit::default(),
        };
        audit.visit_signature(signature);
        audit.visit_block(block);
        if audit.has_core_type {
            for violation in audit.decoder.violations {
                self.failures
                    .push(format!("{}: {}: {violation}", self.path, signature.ident));
            }
        }
    }

    fn inspect_type(&mut self, name: &str) {
        if self.core_types.contains(name)
            || has_numeric_version_suffix(name) && name.starts_with("Harness")
        {
            self.failures.push(format!(
                "{}: native code must import Core Harness types, not redeclare {name}",
                self.path
            ));
        }
    }
}
impl<'ast> Visit<'ast> for NativeBoundaryAudit<'_> {
    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        if has_cfg_test(&item.attrs) {
            return;
        }
        fn imports(tree: &syn::UseTree, prefix: &str, paths: &mut Vec<String>) {
            match tree {
                syn::UseTree::Path(path) => {
                    imports(&path.tree, &format!("{prefix}{}::", path.ident), paths)
                }
                syn::UseTree::Name(name) => paths.push(format!("{prefix}{}", name.ident)),
                syn::UseTree::Rename(name) => paths.push(format!("{prefix}{}", name.ident)),
                syn::UseTree::Glob(_) => paths.push(format!("{prefix}*")),
                syn::UseTree::Group(group) => {
                    for tree in &group.items {
                        imports(tree, prefix, paths);
                    }
                }
            }
        }
        let mut paths = Vec::new();
        imports(&item.tree, "", &mut paths);
        for path in paths {
            if path.starts_with("context_core::harness::") {
                self.core_imports += 1;
            }
            if path.ends_with("::decode_current_json")
                && path != "context_core::harness::decode_current_json"
            {
                self.failures.push(format!(
                    "{}: canonical decoder import must point to Core",
                    self.path
                ));
            }
        }
        visit::visit_item_use(self, item);
    }

    fn visit_item_mod(&mut self, item: &'ast ItemMod) {
        if !has_cfg_test(&item.attrs) {
            visit::visit_item_mod(self, item);
        }
    }
    fn visit_item_fn(&mut self, item: &'ast ItemFn) {
        if has_cfg_test(&item.attrs) {
            return;
        }
        self.inspect_role_decoder(&item.sig, &item.block);
        let previous = std::mem::replace(&mut self.function, item.sig.ident.to_string());
        if self.function == "decode_current_json" {
            self.failures.push(format!(
                "{}: native runtime must not duplicate the Core decoder",
                self.path
            ));
        }
        if self.function == "read_harness_json" {
            let mut shape = DecoderShape::default();
            shape.visit_item_fn(item);
            if !shape.calls.contains("decode_current_json") {
                self.failures.push(format!(
                    "{}: read_harness_json must call decode_current_json",
                    self.path
                ));
            }
        }
        visit::visit_item_fn(self, item);
        self.function = previous;
    }
    fn visit_impl_item_fn(&mut self, item: &'ast ImplItemFn) {
        if has_cfg_test(&item.attrs) {
            return;
        }
        self.inspect_role_decoder(&item.sig, &item.block);
        visit::visit_impl_item_fn(self, item);
    }
    fn visit_item_const(&mut self, item: &'ast ItemConst) {
        if has_cfg_test(&item.attrs) {
            return;
        }
        if item.ident == "HARNESS_SCHEMA_VERSION"
            || item.ident.to_string().contains("HARNESS")
                && item.ident.to_string().contains("VERSION")
        {
            self.failures.push(format!(
                "{}: native runtime must use the Core schema constant",
                self.path
            ));
        }
        visit::visit_item_const(self, item);
    }
    fn visit_item_struct(&mut self, item: &'ast ItemStruct) {
        if !has_cfg_test(&item.attrs) {
            self.inspect_type(&item.ident.to_string());
            visit::visit_item_struct(self, item);
        }
    }
    fn visit_item_enum(&mut self, item: &'ast ItemEnum) {
        if !has_cfg_test(&item.attrs) {
            self.inspect_type(&item.ident.to_string());
            visit::visit_item_enum(self, item);
        }
    }
    fn visit_item_type(&mut self, item: &'ast ItemType) {
        if !has_cfg_test(&item.attrs) {
            self.inspect_type(&item.ident.to_string());
            visit::visit_item_type(self, item);
        }
    }
    fn visit_expr_call(&mut self, expression: &'ast syn::ExprCall) {
        if let Expr::Path(path) = expression.func.as_ref() {
            let segments = path
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>();
            if segments
                .last()
                .is_some_and(|name| name == "decode_current_json")
            {
                self.canonical_decoder_calls += 1;
            }
            if segments == ["meenseek_ontology", "native_harness", "run"] {
                self.main_dispatches += 1;
            }
        }
        visit::visit_expr_call(self, expression);
    }
}

// Native role helpers that name a Core type participate in the Harness boundary. Ordinary
// application/domain JSON helpers, and the separately validated context commit codec, retain
// their own decoding contracts.
struct NativeRoleDecoderAudit<'a> {
    core_types: &'a BTreeSet<String>,
    has_core_type: bool,
    decoder: HarnessSourceAudit,
}
impl<'ast> Visit<'ast> for NativeRoleDecoderAudit<'_> {
    fn visit_type_path(&mut self, path: &'ast TypePath) {
        if path
            .path
            .segments
            .last()
            .is_some_and(|segment| self.core_types.contains(&segment.ident.to_string()))
        {
            self.has_core_type = true;
        }
        visit::visit_type_path(self, path);
    }
    fn visit_expr_call(&mut self, expression: &'ast syn::ExprCall) {
        self.decoder.inspect_decoder_call(&expression.func);
        visit::visit_expr_call(self, expression);
    }
}

fn assert_current_command_set(commands: &BTreeSet<String>, failures: &mut Failures) {
    let expected = CURRENT_HARNESS_COMMANDS
        .iter()
        .map(|command| (*command).to_owned())
        .collect::<BTreeSet<_>>();
    if *commands != expected {
        failures.push(format!(
            "Harness command set must be exactly {expected:?}, found {commands:?}"
        ));
    }
}

#[cfg(test)]
fn assert_harness_runtime_source(
    relative_path: &str,
    text: &str,
    failures: &mut Failures,
) -> usize {
    let syntax = match syn::parse_file(text) {
        Ok(syntax) => syntax,
        Err(error) => {
            failures.push(format!(
                "Failed to parse Harness runtime source {relative_path}: {error}"
            ));
            return 0;
        }
    };
    let mut audit = HarnessSourceAudit {
        dispatch_function: if relative_path == "src/native_harness.rs" {
            "run"
        } else {
            "run_harness_command"
        },
        ..HarnessSourceAudit::default()
    };
    audit.visit_file(&syntax);
    if relative_path == "src/native_harness.rs" {
        let expected = CURRENT_HARNESS_COMMANDS
            .iter()
            .map(|command| (*command).to_owned())
            .collect::<BTreeSet<_>>();
        if audit.harness_commands != expected {
            audit.violations.push(format!(
                "Harness command set must be exactly {expected:?}, found {:?}",
                audit.harness_commands
            ));
        }
    }
    for violation in audit.violations {
        failures.push(format!("{relative_path}: {violation}"));
    }
    audit.version_constants
}

#[derive(Default)]
struct HarnessSourceAudit {
    dispatch_function: &'static str,
    harness_commands: BTreeSet<String>,
    version_constants: usize,
    function_context: Vec<String>,
    violations: Vec<String>,
}

impl HarnessSourceAudit {
    fn current_function(&self) -> Option<&str> {
        self.function_context.last().map(String::as_str)
    }

    fn inspect_type_identifier(&mut self, identifier: &str) {
        if has_numeric_version_suffix(identifier) {
            self.violations.push(format!(
                "version-suffixed Harness type `{identifier}` is forbidden; replace the current type in place"
            ));
        }
        self.inspect_compatibility_identifier(identifier);
    }

    fn inspect_compatibility_identifier(&mut self, identifier: &str) {
        let lowercase = identifier.to_ascii_lowercase();
        for forbidden in [
            "legacy",
            "compat",
            "bridge",
            "cutover",
            "migrator",
            "shim",
            "deprecated",
            "superseded",
            "adapterregistry",
            "adapter_registry",
        ] {
            if lowercase.contains(forbidden) {
                self.violations.push(format!(
                    "Harness compatibility identifier `{identifier}` is forbidden ({forbidden})"
                ));
            }
        }
        if contains_identifier_component(identifier, "old") {
            self.violations.push(format!(
                "Harness compatibility identifier `{identifier}` is forbidden (old)"
            ));
        }
    }

    fn inspect_decoder_call(&mut self, expression: &Expr) {
        let Expr::Path(path) = expression else {
            return;
        };
        let Some(function) = path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
        else {
            return;
        };
        if !matches!(
            function.as_str(),
            "from_slice" | "from_str" | "from_reader" | "from_value"
        ) {
            return;
        }
        let allowed = matches!(
            (self.current_function(), function.as_str()),
            (Some("decode_current_json"), "from_slice" | "from_value")
        );
        if !allowed {
            self.violations.push(format!(
                "direct deserializer `{function}` bypasses decode_current_json"
            ));
        }
    }

    fn inspect_serde_attribute(&mut self, attribute: &Attribute) {
        if !attribute.path().is_ident("serde") {
            return;
        }
        let Meta::List(list) = &attribute.meta else {
            return;
        };
        let options = list.tokens.to_string();
        for forbidden in [
            "alias",
            "default",
            "flatten",
            "untagged",
            "deserialize_with",
            "serialize_with",
            "skip",
            "skip_deserializing",
            "skip_serializing",
            "skip_serializing_if",
            "with",
            "other",
        ] {
            if contains_identifier_word(&options, forbidden) {
                self.violations.push(format!(
                    "serde `{forbidden}` can widen or normalize the accepted Harness shape and is forbidden"
                ));
            }
        }
    }

    fn enter_function(&mut self, identifier: &str) {
        self.inspect_compatibility_identifier(identifier);
        self.function_context.push(identifier.to_owned());
    }
}

impl<'ast> Visit<'ast> for HarnessSourceAudit {
    fn visit_expr_match(&mut self, expression: &'ast syn::ExprMatch) {
        fn collect(pattern: &syn::Pat, commands: &mut BTreeSet<String>) {
            match pattern {
                syn::Pat::Lit(literal) => {
                    if let syn::Lit::Str(value) = &literal.lit {
                        commands.insert(value.value());
                    }
                }
                syn::Pat::Or(pattern) => {
                    for case in &pattern.cases {
                        collect(case, commands);
                    }
                }
                _ => {}
            }
        }
        if self.current_function() == Some(self.dispatch_function) {
            for arm in &expression.arms {
                collect(&arm.pat, &mut self.harness_commands);
            }
        }
        visit::visit_expr_match(self, expression);
    }

    fn visit_item_const(&mut self, item: &'ast ItemConst) {
        if has_cfg_test(&item.attrs) {
            return;
        }
        let identifier = item.ident.to_string();
        if identifier.contains("VERSION") {
            if identifier == "HARNESS_SCHEMA_VERSION" {
                self.version_constants += 1;
            } else {
                self.violations.push(format!(
                    "independent version constant `{identifier}` is forbidden"
                ));
            }
        }
        visit::visit_item_const(self, item);
    }

    fn visit_item_struct(&mut self, item: &'ast ItemStruct) {
        if !has_cfg_test(&item.attrs) {
            self.inspect_type_identifier(&item.ident.to_string());
            visit::visit_item_struct(self, item);
        }
    }

    fn visit_item_enum(&mut self, item: &'ast ItemEnum) {
        if !has_cfg_test(&item.attrs) {
            self.inspect_type_identifier(&item.ident.to_string());
            visit::visit_item_enum(self, item);
        }
    }

    fn visit_item_type(&mut self, item: &'ast ItemType) {
        if !has_cfg_test(&item.attrs) {
            self.inspect_type_identifier(&item.ident.to_string());
            visit::visit_item_type(self, item);
        }
    }

    fn visit_item_union(&mut self, item: &'ast ItemUnion) {
        if !has_cfg_test(&item.attrs) {
            self.inspect_type_identifier(&item.ident.to_string());
            visit::visit_item_union(self, item);
        }
    }

    fn visit_item_mod(&mut self, item: &'ast ItemMod) {
        if !has_cfg_test(&item.attrs) {
            self.inspect_compatibility_identifier(&item.ident.to_string());
            visit::visit_item_mod(self, item);
        }
    }

    fn visit_item_fn(&mut self, item: &'ast ItemFn) {
        if has_cfg_test(&item.attrs) {
            return;
        }
        self.enter_function(&item.sig.ident.to_string());
        visit::visit_item_fn(self, item);
        self.function_context.pop();
    }

    fn visit_impl_item_fn(&mut self, item: &'ast ImplItemFn) {
        if has_cfg_test(&item.attrs) {
            return;
        }
        self.enter_function(&item.sig.ident.to_string());
        visit::visit_impl_item_fn(self, item);
        self.function_context.pop();
    }

    fn visit_item_impl(&mut self, item: &'ast ItemImpl) {
        if has_cfg_test(&item.attrs) {
            return;
        }
        if item
            .trait_
            .as_ref()
            .and_then(|(_, path, _)| path.segments.last())
            .is_some_and(|segment| segment.ident == "Deserialize")
        {
            self.violations.push(
                "manual Deserialize implementation is forbidden for Harness runtime types"
                    .to_owned(),
            );
        }
        visit::visit_item_impl(self, item);
    }

    fn visit_field(&mut self, field: &'ast Field) {
        if field.ident.as_ref().is_some_and(|identifier| {
            matches!(
                identifier.to_string().as_str(),
                "schema_version" | "contract_version"
            )
        }) {
            self.violations.push(
                "parallel `schema_version` or `contract_version` field is forbidden".to_owned(),
            );
        }
        visit::visit_field(self, field);
    }

    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        self.inspect_serde_attribute(attribute);
        visit::visit_attribute(self, attribute);
    }

    fn visit_expr_call(&mut self, expression: &'ast syn::ExprCall) {
        self.inspect_decoder_call(&expression.func);
        visit::visit_expr_call(self, expression);
    }

    fn visit_type_path(&mut self, path: &'ast TypePath) {
        let segments = path
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        if segments.ends_with(&["serde_json".to_owned(), "Value".to_owned()])
            && self.current_function() != Some("decode_current_json")
        {
            self.violations.push(
                "serde_json::Value may only be used by the canonical decoder or raw identity binding"
                    .to_owned(),
            );
        }
        visit::visit_type_path(self, path);
    }

    fn visit_lit_str(&mut self, literal: &'ast LitStr) {
        let value = literal.value().to_ascii_lowercase();
        if contains_dash_v_digit(&value) {
            self.violations
                .push("versioned Harness command literal is forbidden".to_owned());
        }
        if self.current_function() == Some(self.dispatch_function)
            && [
                "legacy", "compat", "bridge", "cutover", "migrate", "old", "fallback",
            ]
            .iter()
            .any(|forbidden| value.contains(forbidden))
        {
            self.violations
                .push("compatibility Harness command literal is forbidden".to_owned());
        }
        visit::visit_lit_str(self, literal);
    }
}

fn contains_dash_v_digit(text: &str) -> bool {
    text.as_bytes()
        .windows(3)
        .any(|window| window[0] == b'-' && window[1] == b'v' && window[2].is_ascii_digit())
}

fn has_numeric_version_suffix(name: &str) -> bool {
    name.rsplit_once('V').is_some_and(|(_, suffix)| {
        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn contains_identifier_word(text: &str, expected: &str) -> bool {
    text.split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .any(|word| word == expected)
}

fn contains_identifier_component(identifier: &str, expected: &str) -> bool {
    let lowercase = identifier.to_ascii_lowercase();
    lowercase == expected
        || lowercase.starts_with(&format!("{expected}_"))
        || lowercase.ends_with(&format!("_{expected}"))
        || lowercase.contains(&format!("_{expected}_"))
        || identifier.starts_with(&capitalize_ascii(expected))
        || identifier.contains(&capitalize_ascii(expected))
}

fn capitalize_ascii(value: &str) -> String {
    let mut bytes = value.as_bytes().to_vec();
    if let Some(first) = bytes.first_mut() {
        first.make_ascii_uppercase();
    }
    String::from_utf8(bytes).expect("ASCII compatibility identifier must remain UTF-8")
}

fn has_cfg_test(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("cfg")
            && matches!(&attribute.meta, Meta::List(list) if list.tokens.to_string() == "test")
    })
}

fn assert_harness_operating_contract(repo_root: &Path, failures: &mut Failures) {
    let relative_path = "vault/profile/preferences/agent-harness-operating-plan.md";
    let text = match read_text(repo_root, relative_path) {
        Ok(text) => text,
        Err(error) => {
            failures.push(error.to_string());
            return;
        }
    };
    let frontmatter = match ParsedFrontmatter::from_markdown(&text) {
        Ok(Some(frontmatter)) => frontmatter,
        Ok(None) => {
            failures.push(format!("Missing required frontmatter in {relative_path}"));
            return;
        }
        Err(error) => {
            failures.push(format!("Invalid frontmatter in {relative_path}: {error}"));
            return;
        }
    };
    match frontmatter.optional_scalar_text("type") {
        Ok(Some(actual)) if actual == "purpose" => {}
        Ok(actual) => failures.push(format!(
            "Harness operating plan in {relative_path} must remain a `purpose` document, got {actual:?}"
        )),
        Err(error) => failures.push(format!("Invalid frontmatter in {relative_path}: {error}")),
    }
    for field in ["status", "activation", "plan_version", "max_planned_roles"] {
        match frontmatter.optional_scalar_text(field) {
            Ok(None) => {}
            Ok(Some(actual)) => failures.push(format!(
                "Harness operating plan in {relative_path} must not duplicate mutable `{field}` state, got `{actual}`"
            )),
            Err(error) => failures.push(format!("Invalid frontmatter in {relative_path}: {error}")),
        }
    }
    if !text.contains("호환 계층이나 전환 registry를 만들지 않는다") {
        failures.push(format!(
            "Harness operating plan in {relative_path} must preserve the zero-compatibility rule"
        ));
    }
}

fn assert_derived_directories_are_ignored(repo_root: &Path, failures: &mut Failures) {
    let gitignore = match read_text(repo_root, ".gitignore") {
        Ok(text) => text,
        Err(error) => {
            failures.push(error.to_string());
            return;
        }
    };
    for prefix in DERIVED_PREFIXES {
        let ignored_path = format!("/{prefix}");
        if !gitignore.contains(&ignored_path) {
            failures.push(format!(
                "Derived vault directory is not ignored: {ignored_path}"
            ));
        }
    }
}

fn assert_no_source_markdown_in_raw_areas(repo_root: &Path, failures: &mut Failures) -> Result<()> {
    for raw_root in ["vault/conversations", "vault/rules"] {
        let absolute_root = repo_root.join(raw_root);
        if !absolute_root.is_dir() {
            continue;
        }
        for file_path in list_files(&absolute_root)? {
            if file_path
                .extension()
                .is_some_and(|extension| extension == "md")
            {
                failures.push(format!(
                    "Raw or retired area must not contain active Markdown: {}",
                    relative_to_root(repo_root, &file_path)
                ));
            }
        }
    }
    Ok(())
}

fn assert_desktop_manifest_shape(manifest: &DesktopManifest, failures: &mut Failures) {
    if manifest.retired_root != DEFAULT_RETIRED_ROOT {
        failures.push(format!(
            "Unexpected retiredRoot in manifest: {}",
            manifest.retired_root
        ));
    }
    if manifest.total_file_count != 463 {
        failures.push(format!(
            "Unexpected totalFileCount in manifest: {}",
            manifest.total_file_count
        ));
    }
    if manifest.git_file_count != 412 {
        failures.push(format!(
            "Unexpected gitFileCount in manifest: {}",
            manifest.git_file_count
        ));
    }
    if manifest.operational_markdown.len() != 48 {
        failures.push(format!(
            "Manifest operationalMarkdown count changed: expected 48, got {}",
            manifest.operational_markdown.len()
        ));
    }
}

fn assert_desktop_audit_covers_manifest(
    repo_root: &Path,
    manifest: &DesktopManifest,
    failures: &mut Failures,
) -> Result<()> {
    let mapped_sources = mapped_retired_sources(repo_root)?;
    let expected_sources = manifest
        .operational_markdown
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();

    for source in &expected_sources {
        if !mapped_sources.contains(source) {
            failures.push(format!("Manifest source is not mapped in audit: {source}"));
        }
    }
    for source in &mapped_sources {
        if !expected_sources.contains(source) {
            failures.push(format!(
                "Audit maps a source not listed in manifest: {source}"
            ));
        }
    }
    Ok(())
}

fn assert_desktop_repo_targets_exist(repo_root: &Path, failures: &mut Failures) -> Result<()> {
    for target in parse_repo_local_targets(repo_root)? {
        if !repo_root.join(&target).exists() {
            failures.push(format!("Missing repo-local audit target: {target}"));
        }
    }
    Ok(())
}

fn assert_no_stale_operational_references(repo_root: &Path, failures: &mut Failures) -> Result<()> {
    let roots = ["AGENTS.md", "README.md", "vault", "docs"];
    let skipped_prefixes = [
        "docs/migration/",
        "vault/index/",
        "vault/indexes/",
        "vault/embeddings/",
        "vault/vector-store/",
        "vault/.cache/",
        "vault/cache/",
        "vault/tmp/",
        "vault/exports/",
    ];
    let stale_patterns = [
        "/Users/john/Desktop/docs",
        "docs/.claude",
        ".claude/rules",
        ".claude/agents",
        "docs.retired",
        "legacy-desktop-docs",
    ];

    for root in roots {
        let absolute_root = repo_root.join(root);
        if !absolute_root.exists() {
            continue;
        }
        let files = if absolute_root.is_dir() {
            list_files(&absolute_root)?
        } else {
            vec![absolute_root]
        };

        for file_path in files {
            let relative_path = relative_to_root(repo_root, &file_path);
            if is_macos_metadata_path(&relative_path)
                || has_any_prefix(&relative_path, &skipped_prefixes)
            {
                continue;
            }
            if !is_text_file_for_reference_scan(&file_path) {
                continue;
            }
            let text = fs::read_to_string(&file_path)
                .map_err(|source| ContextVaultError::io("read file", &file_path, source))?;
            for pattern in stale_patterns {
                if text.contains(pattern) {
                    failures.push(format!(
                        "Stale retired path reference in {relative_path}: {pattern}"
                    ));
                }
            }
        }
    }
    Ok(())
}

fn verify_retired_root_if_available(
    repo_root: &Path,
    retired_root: &Path,
    manifest: &DesktopManifest,
    failures: &mut Failures,
) -> Result<()> {
    if !retired_root.exists() {
        println!(
            "Retired source directory not found; skipped local source comparison: {}",
            retired_root.display()
        );
        return Ok(());
    }

    let files = list_files(retired_root)?;
    let retired_root_posix = to_posix(retired_root);
    let git_prefix = format!("{retired_root_posix}/.git/");
    let non_git_files = files
        .iter()
        .filter(|file_path| !to_posix(file_path).starts_with(&git_prefix))
        .collect::<Vec<_>>();
    let markdown_files = non_git_files
        .iter()
        .filter(|file_path| {
            file_path
                .extension()
                .is_some_and(|extension| extension == "md")
        })
        .copied()
        .collect::<Vec<_>>();
    let non_markdown_files = non_git_files
        .iter()
        .filter(|file_path| {
            file_path
                .extension()
                .is_none_or(|extension| extension != "md")
        })
        .copied()
        .collect::<Vec<_>>();

    if files.len() != manifest.total_file_count {
        failures.push(format!(
            "Retired file count changed: expected {}, got {}",
            manifest.total_file_count,
            files.len()
        ));
    }
    if files.len() - non_git_files.len() != manifest.git_file_count {
        failures.push(format!(
            "Retired .git file count changed: expected {}, got {}",
            manifest.git_file_count,
            files.len() - non_git_files.len()
        ));
    }
    if markdown_files.len() != manifest.operational_markdown.len() {
        failures.push(format!(
            "Retired operational Markdown count changed: expected {}, got {}",
            manifest.operational_markdown.len(),
            markdown_files.len()
        ));
    }
    if non_markdown_files.len() != manifest.non_markdown_local_files.len() {
        failures.push(format!(
            "Retired non-Markdown local/system file count changed: expected {}, got {}",
            manifest.non_markdown_local_files.len(),
            non_markdown_files.len()
        ));
    }

    assert_retired_mappings(repo_root, retired_root, &markdown_files, manifest, failures)?;

    let manifest_non_markdown = manifest
        .non_markdown_local_files
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    for file_path in non_markdown_files {
        let relative_path = relative_to_root(retired_root, file_path);
        if !manifest_non_markdown.contains(&relative_path) {
            failures.push(format!(
                "Unexpected retired non-Markdown file: {relative_path}"
            ));
        }
    }
    Ok(())
}

fn assert_retired_mappings(
    repo_root: &Path,
    retired_root: &Path,
    markdown_files: &[&PathBuf],
    manifest: &DesktopManifest,
    failures: &mut Failures,
) -> Result<()> {
    let rule_mappings = parse_section_mappings(repo_root, "Rule Mapping")?;
    let agent_mappings = parse_section_mappings(repo_root, "Agent And Overview Mapping")?;
    let mapped_sources = mapped_retired_sources(repo_root)?;
    let retired_markdown_sources = markdown_files
        .iter()
        .map(|file_path| relative_to_root(retired_root, file_path))
        .collect::<BTreeSet<_>>();

    for source in &manifest.operational_markdown {
        if !retired_markdown_sources.contains(source) {
            failures.push(format!(
                "Manifest source is missing from retired source directory: {source}"
            ));
        }
    }
    for source in &retired_markdown_sources {
        if !manifest.operational_markdown.contains(source) {
            failures.push(format!(
                "Retired Markdown source is missing from manifest: {source}"
            ));
        }
    }
    for source in &mapped_sources {
        if !retired_markdown_sources.contains(source) {
            failures.push(format!(
                "Audit maps a retired source that is missing on disk: {source}"
            ));
        }
    }

    assert_mapped_content(repo_root, retired_root, "rule", &rule_mappings, failures)?;
    assert_mapped_content(
        repo_root,
        retired_root,
        "agent/overview",
        &agent_mappings,
        failures,
    )
}

fn assert_mapped_content(
    repo_root: &Path,
    retired_root: &Path,
    label: &str,
    mappings: &[(String, Vec<String>)],
    failures: &mut Failures,
) -> Result<()> {
    for (source, targets) in mappings {
        assert_one_mapped_content(repo_root, retired_root, label, source, targets, failures)?;
    }
    Ok(())
}

fn assert_one_mapped_content(
    repo_root: &Path,
    retired_root: &Path,
    label: &str,
    source: &str,
    targets: &[String],
    failures: &mut Failures,
) -> Result<()> {
    let source_path = retired_root.join(source);
    if !source_path.exists() {
        failures.push(format!("{label} source missing: {source}"));
        return Ok(());
    }
    if targets.is_empty() {
        failures.push(format!("{label} mapping has no target: {source}"));
        return Ok(());
    }
    if targets.iter().collect::<BTreeSet<_>>().len() != targets.len() {
        failures.push(format!(
            "{label} mapping repeats a target: {source} -> {}",
            targets.join(", ")
        ));
        return Ok(());
    }

    let source_text = fs::read_to_string(&source_path)
        .map_err(|error| ContextVaultError::io("read file", &source_path, error))?;
    let Some(target_texts) = read_mapped_target_texts(repo_root, label, targets, failures)? else {
        return Ok(());
    };
    assert_mapped_section_coverage(
        label,
        source,
        targets,
        &source_text,
        &target_texts,
        failures,
    );
    Ok(())
}

fn read_mapped_target_texts(
    repo_root: &Path,
    label: &str,
    targets: &[String],
    failures: &mut Failures,
) -> Result<Option<Vec<String>>> {
    let mut target_texts = Vec::new();
    for target in targets {
        let target_path = repo_root.join(target);
        if !target_path.is_file() {
            failures.push(format!("{label} target missing: {target}"));
            continue;
        }
        target_texts.push(
            fs::read_to_string(&target_path)
                .map_err(|error| ContextVaultError::io("read file", &target_path, error))?,
        );
    }
    Ok((target_texts.len() == targets.len()).then_some(target_texts))
}

fn assert_mapped_section_coverage(
    label: &str,
    source: &str,
    targets: &[String],
    source_text: &str,
    target_texts: &[String],
    failures: &mut Failures,
) {
    let source_sections = markdown_sections(source_text);
    let source_headings = source_sections
        .iter()
        .map(|section| section.heading.clone())
        .collect::<BTreeSet<_>>();
    let target_sections = target_texts
        .iter()
        .map(|text| markdown_sections(text))
        .collect::<Vec<_>>();
    let target_headings = target_sections
        .iter()
        .map(|sections| {
            sections
                .iter()
                .map(|section| section.heading.clone())
                .collect::<BTreeSet<_>>()
        })
        .collect::<Vec<_>>();
    let target_label = targets.join(", ");

    if source_sections.is_empty() {
        let source_line_count = source_text.lines().count();
        let target_line_count = target_texts
            .iter()
            .map(|text| text.lines().count())
            .sum::<usize>();
        if target_line_count * 100 < source_line_count * 65 {
            failures.push(format!(
                "{label} target is materially shorter than source: {source} -> {target_label} ({source_line_count} -> {target_line_count})"
            ));
        }
    } else {
        assert_matched_section_lengths(
            label,
            source,
            &target_label,
            &source_sections,
            &target_sections,
            &source_headings,
            failures,
        );
    }

    if targets.len() > 1 && !source_headings.is_empty() {
        for (target, headings) in targets.iter().zip(&target_headings) {
            if source_headings.is_disjoint(headings) {
                failures.push(format!(
                    "{label} split target preserves no source section heading: {source} -> {target}"
                ));
            }
        }
    }
}

fn assert_matched_section_lengths(
    label: &str,
    source: &str,
    target_label: &str,
    source_sections: &[MarkdownSection],
    target_sections: &[Vec<MarkdownSection>],
    source_headings: &BTreeSet<String>,
    failures: &mut Failures,
) {
    let mut source_lengths = BTreeMap::<String, Vec<usize>>::new();
    for section in source_sections {
        source_lengths
            .entry(section.heading.clone())
            .or_default()
            .push(section.line_count);
    }
    let mut target_lengths = BTreeMap::<String, Vec<usize>>::new();
    for section in target_sections.iter().flatten() {
        if source_headings.contains(&section.heading) {
            target_lengths
                .entry(section.heading.clone())
                .or_default()
                .push(section.line_count);
        }
    }

    let source_line_count = source_sections
        .iter()
        .map(|section| section.line_count)
        .sum::<usize>();
    let mut matched_target_line_count = 0;
    for (heading, source_occurrences) in &mut source_lengths {
        source_occurrences.sort_unstable_by(|left, right| right.cmp(left));
        let target_occurrences = target_lengths.entry(heading.clone()).or_default();
        target_occurrences.sort_unstable_by(|left, right| right.cmp(left));
        if target_occurrences.len() < source_occurrences.len() {
            failures.push(format!(
                "{label} heading occurrence missing from mapped targets: {source} -> {target_label}: {heading} ({} -> {})",
                source_occurrences.len(),
                target_occurrences.len()
            ));
        }
        matched_target_line_count += source_occurrences
            .iter()
            .zip(target_occurrences.iter())
            .map(|(source_lines, target_lines)| (*source_lines).min(*target_lines))
            .sum::<usize>();
    }
    if matched_target_line_count * 100 < source_line_count * 65 {
        failures.push(format!(
            "{label} source-matched target sections are materially shorter than the source sections: {source} -> {target_label} ({source_line_count} -> {matched_target_line_count})"
        ));
    }
}

fn assert_retired_raw_tree_removed(repo_root: &Path, failures: &mut Failures) {
    if repo_root.join(RETIRED_RAW_ROOT).exists() {
        failures.push(format!(
            "Retired raw source tree must not be active: {RETIRED_RAW_ROOT}"
        ));
    }
}

fn parse_section_mappings(
    repo_root: &Path,
    section_title: &str,
) -> Result<Vec<(String, Vec<String>)>> {
    let text = read_text(repo_root, DESKTOP_AUDIT_PATH)?;
    let mut pairs = Vec::new();
    let mut in_section = false;
    let mut section_count = 0;
    let mut fence = None;
    let expected_heading = format!("## {section_title}");

    for line in text.lines() {
        if let Some((marker, minimum_length)) = fence {
            if is_closing_fence(line, marker, minimum_length) {
                fence = None;
            }
            continue;
        }
        if let Some(marker) = opening_fence_marker(line) {
            fence = Some(marker);
            continue;
        }
        if let Some(heading) = markdown_heading_line(line) {
            if heading == expected_heading {
                section_count += 1;
                in_section = true;
                continue;
            }
            if in_section && heading.starts_with("## ") && !heading.starts_with("###") {
                in_section = false;
            }
        }
        if !in_section || !line.starts_with("| `") || line.contains("---") {
            continue;
        }

        let cells = line
            .trim()
            .trim_start_matches('|')
            .trim_end_matches('|')
            .split('|')
            .map(str::trim)
            .collect::<Vec<_>>();
        if cells.len() < 2 {
            continue;
        }
        if let Some(source) = first_backtick_value(cells[0]) {
            let targets = backtick_values(cells[1]);
            if !targets.is_empty() {
                pairs.push((source, targets));
            }
        }
    }
    if section_count != 1 {
        return Err(ContextVaultError::invalid_input(format!(
            "migration audit must contain exactly one `{expected_heading}` section, found {section_count}"
        )));
    }
    Ok(pairs)
}

fn parse_repo_local_targets(repo_root: &Path) -> Result<BTreeSet<String>> {
    let text = read_text(repo_root, DESKTOP_AUDIT_PATH)?;
    let mut targets = BTreeSet::new();

    for line in text.lines() {
        if !line.starts_with("| ") || line.contains("---") {
            continue;
        }
        for value in backtick_values(line) {
            if value.starts_with("vault/") || value == "AGENTS.md" || value == "README.md" {
                targets.insert(value);
            }
        }
    }
    Ok(targets)
}

fn mapped_retired_sources(repo_root: &Path) -> Result<BTreeSet<String>> {
    let mut mapped = parse_section_mappings(repo_root, "Rule Mapping")?
        .into_iter()
        .map(|(source, _)| source)
        .chain(
            parse_section_mappings(repo_root, "Agent And Overview Mapping")?
                .into_iter()
                .map(|(source, _)| source),
        )
        .collect::<BTreeSet<_>>();
    mapped.insert("AGENTS.md".to_owned());
    mapped.insert(".claude/CLAUDE.md".to_owned());
    Ok(mapped)
}

#[derive(Debug, Eq, PartialEq)]
struct MarkdownSection {
    heading: String,
    line_count: usize,
}

fn markdown_sections(text: &str) -> Vec<MarkdownSection> {
    let mut sections = Vec::new();
    let mut current = None::<MarkdownSection>;
    let mut fence = None;
    for line in text.lines() {
        if let Some((marker, minimum_length)) = fence {
            if let Some(section) = &mut current {
                section.line_count += 1;
            }
            if is_closing_fence(line, marker, minimum_length) {
                fence = None;
            }
            continue;
        }
        if let Some(marker) = opening_fence_marker(line) {
            if let Some(section) = &mut current {
                section.line_count += 1;
            }
            fence = Some(marker);
            continue;
        }

        if let Some(heading) = markdown_heading_line(line) {
            if let Some(section) = current.replace(MarkdownSection {
                heading: heading.to_owned(),
                line_count: 1,
            }) {
                sections.push(section);
            }
        } else if let Some(section) = &mut current {
            section.line_count += 1;
        }
    }
    if let Some(section) = current {
        sections.push(section);
    }
    sections
}

fn markdown_heading_line(line: &str) -> Option<&str> {
    let indentation = line
        .as_bytes()
        .iter()
        .take_while(|byte| **byte == b' ')
        .count();
    if indentation > 3 {
        return None;
    }
    let trimmed = line[indentation..].trim_end();
    let marker_count = trimmed.bytes().take_while(|byte| *byte == b'#').count();
    ((2..=6).contains(&marker_count)
        && trimmed
            .as_bytes()
            .get(marker_count)
            .is_some_and(u8::is_ascii_whitespace))
    .then_some(trimmed)
}

fn backtick_values(line: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find('`') {
        let after_start = &rest[start + 1..];
        let Some(end) = after_start.find('`') else {
            break;
        };
        values.push(after_start[..end].to_owned());
        rest = &after_start[end + 1..];
    }
    values
}

fn first_backtick_value(text: &str) -> Option<String> {
    backtick_values(text).into_iter().next()
}

fn list_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(current) = stack.pop() {
        let read_dir = fs::read_dir(&current)
            .map_err(|source| ContextVaultError::io("read directory", &current, source))?;
        for entry in read_dir {
            let entry = entry.map_err(|source| {
                ContextVaultError::io("read directory entry", &current, source)
            })?;
            let path = entry.path();
            let metadata = entry
                .metadata()
                .map_err(|source| ContextVaultError::io("read file metadata", &path, source))?;
            if metadata.is_dir() {
                stack.push(path);
            } else if metadata.is_file() {
                files.push(path);
            }
        }
    }

    files.sort();
    Ok(files)
}

fn read_text(repo_root: &Path, relative_path: &str) -> Result<String> {
    let path = repo_root.join(relative_path);
    fs::read_to_string(&path).map_err(|source| ContextVaultError::io("read file", path, source))
}

fn read_verified_text(repo_root: &Path, relative_path: &str) -> Result<String> {
    let path = repo_root.join(relative_path);
    let mut file =
        verified_repository_file(repo_root, Path::new(relative_path)).map_err(|error| {
            ContextVaultError::invalid_input(format!(
                "failed to safely open `{}`: {error}",
                path.display()
            ))
        })?;
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|source| ContextVaultError::io("read file", path, source))?;
    Ok(text)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let text = fs::read_to_string(path)
        .map_err(|source| ContextVaultError::io("read file", path, source))?;
    serde_json::from_str(&text).map_err(|source| {
        ContextVaultError::invalid_input(format!(
            "failed to parse JSON `{}`: {source}",
            path.display()
        ))
    })
}

fn verified_frontmatter(
    text: &str,
    relative_path: &str,
    failures: &mut Failures,
) -> Option<ParsedFrontmatter> {
    match ParsedFrontmatter::from_markdown(text) {
        Ok(frontmatter) => frontmatter,
        Err(error) => {
            failures.push(format!("Invalid frontmatter in {relative_path}: {error}"));
            None
        }
    }
}

fn fenced_markdown_block(text: &str) -> Option<&str> {
    let start_marker = "```md\n";
    let start = text.find(start_marker)? + start_marker.len();
    let end = text[start..].find("\n```")? + start;
    Some(&text[start..end])
}

fn is_iso_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[8..].iter().all(u8::is_ascii_digit)
}

fn split_path(relative_path: &str) -> Vec<&str> {
    relative_path.split('/').collect()
}

fn has_any_prefix(value: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| value.starts_with(prefix))
}

fn is_markdown_path(relative_path: &str) -> bool {
    Path::new(relative_path)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
}

fn is_macos_metadata_path(relative_path: &str) -> bool {
    Path::new(relative_path)
        .file_name()
        .is_some_and(|file_name| file_name == ".DS_Store")
}

fn relative_to_root(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map_or_else(|_| to_posix(path), to_posix)
}

fn to_posix(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn is_text_file_for_reference_scan(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return true;
    };
    matches!(
        extension,
        "json" | "md" | "rs" | "toml" | "txt" | "yaml" | "yml"
    )
}

#[derive(Debug, Deserialize)]
struct DesktopManifest {
    #[serde(rename = "retiredRoot")]
    retired_root: String,
    #[serde(rename = "totalFileCount")]
    total_file_count: usize,
    #[serde(rename = "gitFileCount")]
    git_file_count: usize,
    #[serde(rename = "nonMarkdownLocalFiles")]
    non_markdown_local_files: Vec<String>,
    #[serde(rename = "operationalMarkdown")]
    operational_markdown: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PageGraphPlacement {
    page_count: usize,
    pages: BTreeMap<String, String>,
    #[serde(default)]
    source_external_links: BTreeMap<String, Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDirectory;

    #[test]
    fn native_harness_architecture_matches_repository() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut failures = Failures::default();
        assert_native_harness_architecture(&root, &mut failures);
        assert!(failures.messages.is_empty(), "{:?}", failures.messages);
    }

    fn native_architecture_fixture() -> TempDirectory {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let fixture = TempDirectory::new("native-architecture");
        for path in CURRENT_HARNESS_RUNTIME_FILES.iter().copied().chain([
            "Cargo.toml",
            "crates/context-core/Cargo.toml",
            "crates/context-core/src/lib.rs",
        ]) {
            let content = read_verified_text(&root, path)
                .expect("actual candidate code inventory must be safely readable");
            let target = fixture.path().join(path);
            fs::create_dir_all(target.parent().expect("inventory file has a parent"))
                .expect("create owned native tree");
            fs::write(target, content).expect("copy actual candidate source");
        }
        fixture
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "the mutation table keeps every native architecture obligation independently exercised"
    )]
    fn native_harness_architecture_rejects_mutated_repository() {
        let mutations = [
            (
                "src/native_harness.rs",
                "",
                "let allowed: &[&str] = match verb.as_str() {",
                "let allowed: &[&str] = match verb.as_str() {\n        \"extra-verb\" => &[],",
            ),
            (
                "crates/context-core/src/harness.rs",
                "\npub const HARNESS_SCHEMA_VERSION: u32 = 8;\n",
                "",
                "",
            ),
            (
                "crates/context-core/src/harness/request.rs",
                "\nconst REQUEST_SCHEMA_VERSION: u32 = 1;\n",
                "",
                "",
            ),
            (
                "crates/context-core/src/harness/request.rs",
                "\nstruct HarnessRequestV2;\n",
                "",
                "",
            ),
            (
                "crates/context-core/src/harness/request.rs",
                "\nimpl<'de> serde::Deserialize<'de> for Manual { fn deserialize<D>(_: D) -> Result<Self, D::Error> { todo!() } }\n",
                "",
                "",
            ),
            (
                "crates/context-core/src/harness/request.rs",
                "\n#[serde(default)] struct Relaxed { value: String }\n",
                "",
                "",
            ),
            (
                "src/native_harness.rs",
                "\nfn bypass(bytes: &[u8]) { let _: HarnessPlan = serde_json::from_slice(bytes).unwrap(); }\n",
                "",
                "",
            ),
            (
                "src/native_role.rs",
                "\nfn bypass_role_plan(bytes: &[u8]) -> context_core::harness::ResolvedHarnessPlan { serde_json::from_slice(bytes).unwrap() }\n",
                "",
                "",
            ),
            ("src/native_harness.rs", "\nstruct HarnessPlan;\n", "", ""),
            (
                "src/lib.rs",
                "\n#[path = \"../crates/context-core/src/harness.rs\"] mod harness;\n",
                "",
                "",
            ),
            (
                "crates/context-core/src/harness.rs",
                "",
                "mod source;",
                "#[path = \"../../../escaped.rs\"] mod source;",
            ),
            ("crates/context-core/src/harness.rs", "", "mod source;", ""),
            (
                "crates/context-core/src/harness.rs",
                "",
                "if input != canonical {",
                "if false {",
            ),
            (
                "src/main.rs",
                "",
                "meenseek_ontology::native_harness::run",
                "other::run",
            ),
        ];
        for (path, appended, before, after) in mutations {
            let fixture = native_architecture_fixture();
            let target = fixture.path().join(path);
            let source = fs::read_to_string(&target).expect("owned source must exist");
            let content = if before.is_empty() {
                format!("{source}{appended}")
            } else {
                assert!(
                    source.contains(before),
                    "mutation anchor missing: {path}: {before}"
                );
                format!("{}{appended}", source.replacen(before, after, 1))
            };
            fs::write(target, content).expect("install owned mutation");
            let mut failures = Failures::default();
            assert_native_harness_architecture(fixture.path(), &mut failures);
            assert!(
                !failures.messages.is_empty(),
                "architecture accepted {path}: {before}: {appended}"
            );
            if path == "src/native_harness.rs" && !before.is_empty() {
                assert!(
                    failures.messages.iter().any(|message| {
                        message.contains("Harness command set must be exactly")
                            && message.contains("extra-verb")
                    }),
                    "the public dispatch mutation must fail the exact command set: {:?}",
                    failures.messages
                );
            }
            if path == "src/native_role.rs" {
                assert!(
                    failures.messages.iter().any(|message| {
                        message.contains("src/native_role.rs: bypass_role_plan")
                            && message.contains("bypasses decode_current_json")
                    }),
                    "the native role mutation must fail its typed decoder boundary: {:?}",
                    failures.messages
                );
            }
        }
        let fixture = native_architecture_fixture();
        let target = fixture.path().join("src/native_role.rs");
        let source = fs::read_to_string(&target).expect("owned native role source must exist");
        fs::write(target, format!("{source}\nfn decode_domain_json(bytes: &[u8]) -> serde_json::Value {{ serde_json::from_slice(bytes).unwrap() }}\n"))
            .expect("install unrelated domain JSON helper");
        let mut failures = Failures::default();
        assert_native_harness_architecture(fixture.path(), &mut failures);
        assert!(
            failures.messages.is_empty(),
            "unrelated domain JSON must retain its own decoder: {:?}",
            failures.messages
        );
        let fixture = native_architecture_fixture();
        fs::remove_file(
            fixture
                .path()
                .join("crates/context-core/src/harness/source.rs"),
        )
        .expect("remove owned inventory module");
        let mut failures = Failures::default();
        assert_native_harness_architecture(fixture.path(), &mut failures);
        assert!(!failures.messages.is_empty());
        let fixture = native_architecture_fixture();
        fs::write(
            fixture
                .path()
                .join("crates/context-core/src/harness/extra.rs"),
            "fn extra() {}\n",
        )
        .expect("add owned inventory escape");
        let mut failures = Failures::default();
        assert_native_harness_architecture(fixture.path(), &mut failures);
        assert!(!failures.messages.is_empty());
    }

    #[test]
    fn active_vault_files_ignore_reusable_save_locks_after_save_and_delete() {
        let temp = TempDirectory::new("verification-reusable-save-lock");
        let vault_root = temp.path().join("vault");
        crate::save_context(
            &vault_root,
            crate::Scope::Profile,
            "preferences/note.md",
            "Note",
            "First body",
        )
        .expect("synthetic context should be saved");
        let note = vault_root.join("profile/preferences/note.md");
        let lock = note.with_file_name(".note.md.save.lock");
        #[cfg(unix)]
        let original_lock_metadata =
            fs::symlink_metadata(&lock).expect("save should leave its lock sidecar");

        let assert_state = |expected_files: &[&str]| {
            let metadata =
                fs::symlink_metadata(&lock).expect("save lock sidecar should remain present");
            assert!(metadata.is_file());
            assert_eq!(metadata.len(), 0);
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt as _;

                assert_eq!(metadata.dev(), original_lock_metadata.dev());
                assert_eq!(metadata.ino(), original_lock_metadata.ino());
                assert_eq!(metadata.nlink(), 1);
            }
            let mut failures = Failures::default();
            let files = active_vault_files(temp.path(), &mut failures)
                .expect("synthetic active files should be listed");
            assert_eq!(files, expected_files);
            assert_document_bundle_attachments(temp.path(), &files, &mut failures);
            assert!(failures.messages.is_empty(), "{:?}", failures.messages);
        };

        assert_state(&["vault/profile/preferences/note.md"]);
        crate::save_context(
            &vault_root,
            crate::Scope::Profile,
            "preferences/note.md",
            "Note",
            "Updated body",
        )
        .expect("synthetic context should be saved again using the existing lock");
        assert_state(&["vault/profile/preferences/note.md"]);
        crate::delete_context(&vault_root, crate::Scope::Profile, "preferences/note.md")
            .expect("synthetic context should be deleted using the existing lock");
        assert!(!note.exists());
        assert_state(&[]);
    }

    #[test]
    fn active_vault_files_ignore_only_empty_context_save_lock_basenames() {
        let temp = TempDirectory::new("verification-save-lock-basenames");
        let directory = temp.path().join("vault/profile/preferences");
        let active_fixtures = [
            (".hidden", ""),
            ("note.lock", ""),
            ("note.md.save.lock", ""),
            (".note.txt.save.lock", ""),
            (".uppercase.MD.save.lock", ""),
            (".note.md.save.lock.bak", ""),
            (".note.md.save.lock.md", "# Real document\n"),
            (".md.save.lock", ""),
            ("..md.save.lock", ""),
            (".note.md.save.lock", "real attachment"),
            (".folder.md.save.lock/nested.md", "# Nested document\n"),
            (".folder.md.save.lock/asset.png", "real attachment"),
        ];
        for (name, contents) in active_fixtures {
            let path = directory.join(name);
            fs::create_dir_all(path.parent().expect("fixture path should have a parent"))
                .expect("fixture directories should be created");
            fs::write(path, contents).expect("active fixture should be written");
        }
        for name in [".empty.md.save.lock", "..hidden.md.save.lock"] {
            fs::write(directory.join(name), []).expect("empty sidecar should be written");
        }

        let mut failures = Failures::default();
        let files = active_vault_files(temp.path(), &mut failures)
            .expect("active vault files should be listed");
        assert!(failures.messages.is_empty());
        let mut expected = active_fixtures
            .iter()
            .map(|(name, _)| format!("vault/profile/preferences/{name}"))
            .collect::<Vec<_>>();
        expected.sort();
        assert_eq!(files, expected);
        assert_document_bundle_attachments(temp.path(), &files, &mut failures);
        let expected_failures = expected
            .iter()
            .filter(|path| !is_markdown_path(path))
            .map(|path| {
                format!(
                    "Active non-Markdown file must live under a document bundle attachments directory: {path}"
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(failures.messages, expected_failures);
    }

    #[cfg(unix)]
    #[test]
    fn save_lock_classification_rejects_symbolic_and_hard_links() {
        use std::os::unix::fs::symlink;

        let temp = TempDirectory::new("verification-linked-save-lock");
        let directory = temp.path().join("vault/profile/preferences");
        fs::create_dir_all(&directory).expect("fixture directory should be created");
        let source = temp.path().join("empty-source");
        fs::write(&source, []).expect("empty link source should be written");
        let symbolic = directory.join(".symbolic.md.save.lock");
        let dangling = directory.join(".dangling.md.save.lock");
        let hard = directory.join(".hard.md.save.lock");
        symlink(&source, &symbolic).expect("symbolic link should be created");
        symlink("missing-source", &dangling).expect("dangling link should be created");
        fs::hard_link(&source, &hard).expect("hard link should be created");

        for path in [&symbolic, &dangling, &hard] {
            assert!(
                !is_empty_context_save_lock(path)
                    .expect("link metadata should be inspected without following symbolic links")
            );
        }
        let mut failures = Failures::default();
        let files = active_vault_files(temp.path(), &mut failures)
            .expect("active files should retain the hard link and skip symbolic links");
        assert_eq!(files, ["vault/profile/preferences/.hard.md.save.lock"]);
        assert_document_bundle_attachments(temp.path(), &files, &mut failures);
        assert_eq!(
            failures.messages,
            [
                "Active non-Markdown file must live under a document bundle attachments directory: vault/profile/preferences/.hard.md.save.lock"
            ]
        );
    }

    #[test]
    fn save_lock_classification_reports_metadata_errors() {
        let temp = TempDirectory::new("verification-save-lock-metadata-error");
        let path = temp.path().join(".missing.md.save.lock");
        let error = is_empty_context_save_lock(&path)
            .expect_err("missing sidecar metadata must not be treated as an excluded file");
        assert!(error.to_string().contains("inspect context save lock"));
    }

    #[test]
    fn active_vault_files_ignore_only_exact_ds_store_basenames() {
        let temp = TempDirectory::new("verification-ds-store");
        let vault_root = temp.path().join("vault");
        let nested_root = vault_root.join("personal/projects/sample");
        fs::create_dir_all(&nested_root).expect("vault fixture directories should be created");
        fs::write(vault_root.join(".DS_Store"), []).expect("root .DS_Store should be written");
        fs::write(nested_root.join(".DS_Store"), []).expect("nested .DS_Store should be written");
        fs::write(nested_root.join(".DS_Store.md"), "# Real document\n")
            .expect("similarly named Markdown should be written");
        fs::write(nested_root.join("notes.DS_Store"), "real attachment")
            .expect("similarly named attachment should be written");

        let mut failures = Failures::default();
        let files = active_vault_files(temp.path(), &mut failures)
            .expect("active vault files should be listed");

        assert!(failures.messages.is_empty());
        assert_eq!(
            files,
            vec![
                "vault/personal/projects/sample/.DS_Store.md",
                "vault/personal/projects/sample/notes.DS_Store",
            ]
        );
    }

    #[test]
    fn stale_reference_scan_ignores_macos_metadata_files() {
        let temp = TempDirectory::new("verification-stale-reference-ds-store");
        let vault_root = temp.path().join("vault");
        fs::create_dir_all(&vault_root).expect("vault fixture directory should be created");
        fs::write(vault_root.join(".DS_Store"), [0xff])
            .expect("binary .DS_Store fixture should be written");

        let mut failures = Failures::default();
        assert_no_stale_operational_references(temp.path(), &mut failures)
            .expect("metadata files should not be read as text");
        assert!(failures.messages.is_empty());
    }

    #[test]
    fn cluml_domain_routing_contract_requires_each_signal_target_association() {
        let engineering_actions = [
            HarnessAction::CodeWrite,
            HarnessAction::CodeReview,
            HarnessAction::DocumentWrite,
            HarnessAction::DocumentReview,
            HarnessAction::Investigation,
            HarnessAction::Design,
        ];
        let mut routes = Vec::new();
        for (signal, target) in CLUML_DOMAIN_ROUTING_CONTRACT {
            let mut policies = vec![(*target).to_owned()];
            if *signal == "jwt" {
                policies.push("vault/work/cluml/rules/cert.md".to_owned());
            }
            routes.push(CompanyDomainRoute {
                signals: vec![(*signal).to_owned()],
                actions: engineering_actions.to_vec(),
                target_kind: CompanyDomainRouteTarget::Any,
                policies,
            });
        }
        routes.push(CompanyDomainRoute {
            signals: CLUML_ISSUE_HUNTING_SIGNALS
                .iter()
                .map(|signal| (*signal).to_owned())
                .collect(),
            actions: vec![HarnessAction::Investigation],
            target_kind: CompanyDomainRouteTarget::Rust,
            policies: CLUML_ISSUE_HUNTING_POLICIES
                .iter()
                .map(|policy| (*policy).to_owned())
                .collect(),
        });

        let mut failures = Failures::default();
        assert_cluml_domain_routing_contract(&routes, &mut failures);

        assert_eq!(failures.messages.len(), 1);
        assert!(
            failures.messages[0].contains("signal `jwt`")
                && failures.messages[0].contains("must have exactly policies")
                && failures.messages[0].contains("vault/work/cluml/rules/cert.md")
        );
    }

    #[test]
    fn consolidated_retired_mapping_requires_distributed_headings_and_unique_targets() {
        let temp = TempDirectory::new("verification-consolidated-mapping");
        let retired_root = temp.path().join("retired");
        let source_path = retired_root.join("source.md");
        let first_target = temp.path().join("first.md");
        let second_target = temp.path().join("second.md");
        fs::create_dir_all(&retired_root).expect("retired fixture directory should be created");
        fs::write(
            &source_path,
            "# Retired\n## Shared Baseline\none\ntwo\n## Domain Routing\nthree\nfour\n",
        )
        .expect("retired source fixture should be written");
        fs::write(&first_target, "# Current\n## Shared Baseline\none\ntwo\n")
            .expect("first target fixture should be written");
        fs::write(&second_target, "## Domain Routing\nthree\nfour\n")
            .expect("second target fixture should be written");

        let mut consolidated_failures = Failures::default();
        assert_mapped_content(
            temp.path(),
            &retired_root,
            "rule",
            &[(
                "source.md".to_owned(),
                vec!["first.md".to_owned(), "second.md".to_owned()],
            )],
            &mut consolidated_failures,
        )
        .expect("consolidated mapping should be checked");
        assert!(consolidated_failures.messages.is_empty());

        let mut single_target_failures = Failures::default();
        assert_mapped_content(
            temp.path(),
            &retired_root,
            "rule",
            &[("source.md".to_owned(), vec!["first.md".to_owned()])],
            &mut single_target_failures,
        )
        .expect("single target mapping should be checked");
        assert!(
            single_target_failures
                .messages
                .iter()
                .any(|message| message.contains("heading occurrence missing"))
        );

        let mut duplicate_failures = Failures::default();
        assert_mapped_content(
            temp.path(),
            &retired_root,
            "rule",
            &[(
                ("source.md").to_owned(),
                vec!["first.md".to_owned(), "first.md".to_owned()],
            )],
            &mut duplicate_failures,
        )
        .expect("duplicate mapping should be checked");
        assert!(duplicate_failures.messages[0].contains("repeats a target"));

        fs::write(
            &second_target,
            "# Unrelated\nlarge\ncontent\nwithout\nsource\nheadings\n",
        )
        .expect("unrelated target fixture should be written");
        let mut unrelated_failures = Failures::default();
        assert_mapped_content(
            temp.path(),
            &retired_root,
            "rule",
            &[(
                ("source.md").to_owned(),
                vec!["first.md".to_owned(), "second.md".to_owned()],
            )],
            &mut unrelated_failures,
        )
        .expect("unrelated split target should be checked");
        assert!(unrelated_failures.messages.iter().any(|message| {
            message.contains("split target preserves no source section heading")
        }));

        fs::write(
            &second_target,
            "# Router\n```md\n## Domain Routing\n```\nlarge\ncontent\n",
        )
        .expect("fenced target decoy should be written");
        let mut fenced_failures = Failures::default();
        assert_mapped_content(
            temp.path(),
            &retired_root,
            "rule",
            &[(
                ("source.md").to_owned(),
                vec!["first.md".to_owned(), "second.md".to_owned()],
            )],
            &mut fenced_failures,
        )
        .expect("fenced target decoy should be checked");
        assert!(fenced_failures.messages.iter().any(|message| {
            message.contains("heading occurrence missing") && message.contains("## Domain Routing")
        }));
    }

    #[test]
    fn unrelated_content_cannot_inflate_split_mapping_coverage() {
        let temp = TempDirectory::new("verification-inflated-split-mapping");
        let retired_root = temp.path().join("retired");
        let source_path = retired_root.join("source.md");
        let first_target = temp.path().join("first.md");
        let second_target = temp.path().join("second.md");
        fs::create_dir_all(&retired_root).expect("retired fixture directory should be created");
        fs::write(
            &source_path,
            "# Retired\n## First\na\nb\nc\nd\ne\nf\ng\nh\ni\n## Second\na\nb\nc\nd\ne\nf\ng\nh\ni\n",
        )
        .expect("long source fixture should be written");
        fs::write(&first_target, "## First\n").expect("short first target should be written");
        fs::write(
            &second_target,
            format!("## Second\n{}", "unrelated\n".repeat(100)),
        )
        .expect("inflated second target should be written");

        let mut failures = Failures::default();
        assert_mapped_content(
            temp.path(),
            &retired_root,
            "rule",
            &[(
                "source.md".to_owned(),
                vec!["first.md".to_owned(), "second.md".to_owned()],
            )],
            &mut failures,
        )
        .expect("inflated split target should be checked");
        assert!(failures.messages.iter().any(|message| {
            message.contains("source-matched target sections are materially shorter")
                && message.contains("(20 -> 11)")
        }));
    }

    #[test]
    fn migration_mapping_sections_are_exact_fence_aware_and_unique() {
        let temp = TempDirectory::new("verification-migration-mapping-sections");
        let audit_path = temp.path().join(DESKTOP_AUDIT_PATH);
        fs::create_dir_all(audit_path.parent().expect("audit should have a parent"))
            .expect("audit fixture directory should be created");
        fs::write(
            &audit_path,
            "```md\n## Rule Mapping\n| `decoy.md` | `decoy-target.md` |\n```\n## Rule Mapping obsolete\n| `obsolete.md` | `obsolete-target.md` |\n## Rule Mapping\n| `source.md` | `target.md` |\n",
        )
        .expect("audit fixture should be written");
        assert_eq!(
            parse_section_mappings(temp.path(), "Rule Mapping")
                .expect("one exact mapping section should parse"),
            vec![("source.md".to_owned(), vec!["target.md".to_owned()])]
        );

        fs::write(
            &audit_path,
            "## Rule Mapping obsolete\n| `source.md` | `target.md` |\n",
        )
        .expect("similarly named audit fixture should be written");
        assert!(parse_section_mappings(temp.path(), "Rule Mapping").is_err());

        fs::write(
            &audit_path,
            "## Rule Mapping\n| `one.md` | `one-target.md` |\n## Rule Mapping\n| `two.md` | `two-target.md` |\n",
        )
        .expect("duplicate audit fixture should be written");
        let error = parse_section_mappings(temp.path(), "Rule Mapping")
            .expect_err("duplicate mapping sections must be rejected");
        assert!(error.to_string().contains("found 2"));
    }

    #[test]
    fn available_retired_root_runs_the_full_mapping_branch() {
        let temp = TempDirectory::new("verification-available-retired-root");
        let retired_root = temp.path().join("retired");
        let retired_rule = retired_root.join(".claude/rules/source.md");
        fs::create_dir_all(retired_rule.parent().expect("rule should have a parent"))
            .expect("retired rule parent should be created");
        fs::write(
            &retired_rule,
            "# Retired\n## Shared Baseline\none\ntwo\n## Domain Routing\nthree\nfour\n```md\n## Output Template\n```\n",
        )
        .expect("retired rule should be written");
        fs::write(retired_root.join("AGENTS.md"), "# Agents\n")
            .expect("retired agents should be written");
        fs::write(retired_root.join(".claude/CLAUDE.md"), "# Claude\n")
            .expect("retired Claude guide should be written");
        fs::write(
            temp.path().join("first.md"),
            "# Current\n## Shared Baseline\none\ntwo\nfive\n",
        )
        .expect("first target should be written");
        fs::write(
            temp.path().join("second.md"),
            "## Domain Routing\nthree\nfour\n",
        )
        .expect("second target should be written");
        let audit_path = temp.path().join(DESKTOP_AUDIT_PATH);
        fs::create_dir_all(audit_path.parent().expect("audit should have a parent"))
            .expect("audit parent should be created");
        fs::write(
            audit_path,
            "## Rule Mapping\n\n| Retired source | Current target |\n| --- | --- |\n| `.claude/rules/source.md` | `first.md`, `second.md` |\n\n## Agent And Overview Mapping\n\n| Retired source | Current target |\n| --- | --- |\n",
        )
        .expect("audit fixture should be written");
        let manifest = DesktopManifest {
            retired_root: DEFAULT_RETIRED_ROOT.to_owned(),
            total_file_count: 3,
            git_file_count: 0,
            non_markdown_local_files: Vec::new(),
            operational_markdown: [
                ".claude/rules/source.md".to_owned(),
                ".claude/CLAUDE.md".to_owned(),
                "AGENTS.md".to_owned(),
            ]
            .into_iter()
            .collect(),
        };

        let mut failures = Failures::default();
        verify_retired_root_if_available(temp.path(), &retired_root, &manifest, &mut failures)
            .expect("available retired root should be checked");
        assert!(failures.messages.is_empty());
    }

    #[test]
    fn routing_graph_discovers_companies_and_rejects_invalid_document_paths() {
        let temp = TempDirectory::new("verification-routing-graph");
        let registry_path = temp
            .path()
            .join("vault/work/common/router/company-registry.md");
        let company_index_path = temp.path().join("vault/work/acme/index.md");
        let project_path = temp.path().join("vault/work/acme/projects/live.md");
        let valid_project_path = temp.path().join("vault/work/acme/projects/valid.md");
        let valid_sibling_path = temp.path().join("valid-sibling.md");
        let policy_path = temp
            .path()
            .join("vault/profile/rules/common-code-quality.md");
        let source_relative_policy_path = temp.path().join("vault/profile/rules/policy.md");
        let fallback_decoy_path = temp.path().join("vault/profile/index.md");
        for parent in [
            registry_path.parent(),
            company_index_path.parent(),
            project_path.parent(),
            valid_project_path.parent(),
            policy_path.parent(),
            source_relative_policy_path.parent(),
            fallback_decoy_path.parent(),
        ] {
            fs::create_dir_all(parent.expect("fixture should have a parent"))
                .expect("fixture directory should be created");
        }
        fs::write(
            temp.path().join("router.md"),
            "`vault/work/common/router/company-registry.md` `vault/profile/rules/policy.md` `[Ignored](inline-code.md)` [Valid sibling](valid-sibling.md \"title ) remains title\") [Missing sibling](sibling.md \"title\")\n    ```\n[Missing after indented marker](after-indented.md)\n```md\n```not-a-close\n[Ignored fenced link](fenced-code.md)\n```\n",
        )
        .expect("root router fixture should be written");
        fs::write(&registry_path, "`vault/work/acme/index.md`\n")
            .expect("registry fixture should be written");
        fs::write(
            &company_index_path,
            "`projects/live.md` `projects/valid.md` `common/rules/engineering-guardrails.md` `profile/rules/common-code-quality.md`\n",
        )
        .expect("company index fixture should be written");
        fs::create_dir(project_path).expect("project-shaped directory fixture should be created");
        fs::write(valid_project_path, "# Valid project\n")
            .expect("valid project fixture should be written");
        fs::write(valid_sibling_path, "# Valid sibling\n")
            .expect("valid sibling fixture should be written");
        fs::write(policy_path, "# Common Code Quality\n")
            .expect("policy fixture should be written");
        fs::write(&source_relative_policy_path, "[Wrong sibling](index.md)\n")
            .expect("source-relative policy fixture should be written");
        fs::write(fallback_decoy_path, "# Fallback decoy\n")
            .expect("fallback decoy fixture should be written");

        let mut failures = Failures::default();
        assert_routing_document_graph_paths_exist_from_roots(
            temp.path(),
            &["router.md"],
            &mut failures,
        );

        assert_eq!(failures.messages.len(), 6);
        assert!(
            failures
                .messages
                .iter()
                .any(|message| message.contains("must be repository-relative"))
        );
        assert!(
            failures
                .messages
                .iter()
                .any(|message| message.contains("common/rules/engineering-guardrails.md"))
        );
        assert!(
            failures
                .messages
                .iter()
                .any(|message| message.contains("sibling.md"))
        );
        assert!(
            failures
                .messages
                .iter()
                .any(|message| message.contains("projects/live.md"))
        );
        assert!(
            failures
                .messages
                .iter()
                .any(|message| message.contains("after-indented.md"))
        );
        assert!(failures.messages.iter().any(|message| {
            message.contains("vault/profile/rules/policy.md") && message.contains("index.md")
        }));
    }

    #[test]
    fn registered_future_company_routes_use_the_shared_parser_and_safe_open() {
        let temp = TempDirectory::new("verification-future-company-routing");
        let registry_path = temp
            .path()
            .join("vault/work/common/router/company-registry.md");
        let routing_path = temp
            .path()
            .join("vault/work/acme/preferences/acme-routing.md");
        for parent in [registry_path.parent(), routing_path.parent()] {
            fs::create_dir_all(parent.expect("fixture should have a parent"))
                .expect("fixture directory should be created");
        }
        fs::write(
            &registry_path,
            "## Company Registry\n\n| Company | Company index |\n| --- | --- |\n| Acme | `vault/work/acme/index.md` |\n",
        )
        .expect("company registry fixture should be written");
        fs::write(
            &routing_path,
            "---\ndomain_routes:\n  - signals: [sample]\n    target_kind: any\n    policies: [\"vault/work/acme/rules/sample.md\"]\n---\n# Routing\n",
        )
        .expect("malformed route fixture should be written");

        let mut malformed_failures = Failures::default();
        assert_company_domain_routing_contracts(temp.path(), &mut malformed_failures);
        assert!(malformed_failures.messages.iter().any(|message| {
            message.contains("vault/work/acme/preferences/acme-routing.md")
                && message.contains("missing field `actions`")
        }));

        fs::write(
            &routing_path,
            "---\ndomain_routes:\n  - signals: [sample]\n    actions: [code-review]\n    target_kind: any\n    policies: [\"vault/work/acme/rules/missing.md\"]\n---\n# Routing\n",
        )
        .expect("valid route fixture should be written");
        let mut missing_policy_failures = Failures::default();
        assert_company_domain_routing_contracts(temp.path(), &mut missing_policy_failures);
        assert!(missing_policy_failures.messages.iter().any(|message| {
            message.contains("vault/work/acme/preferences/acme-routing.md")
                && message.contains("vault/work/acme/rules/missing.md")
                && message.contains("cannot be safely opened")
        }));
    }

    #[cfg(unix)]
    #[test]
    fn routing_file_resolution_rejects_symbolic_and_hard_links() {
        use std::os::unix::fs::symlink;

        let temp = TempDirectory::new("verification-routing-linked-files");
        let regular = temp.path().join("regular.md");
        let linked_source = temp.path().join("linked-source.md");
        let symbolic = temp.path().join("symbolic.md");
        let hard = temp.path().join("hard.md");
        fs::write(&regular, "# Regular\n").expect("regular fixture should be written");
        fs::write(&linked_source, "# Linked\n").expect("link source should be written");
        symlink("linked-source.md", &symbolic).expect("symbolic link should be created");
        fs::hard_link(&linked_source, &hard).expect("hard link should be created");

        assert_eq!(
            canonical_repository_path(temp.path(), Path::new("regular.md")),
            Some("regular.md".to_owned())
        );
        assert!(canonical_repository_path(temp.path(), Path::new("symbolic.md")).is_none());
        assert!(canonical_repository_path(temp.path(), Path::new("hard.md")).is_none());
        assert!(canonical_repository_path(temp.path(), Path::new("linked-source.md")).is_none());
    }

    #[test]
    fn personal_layout_allows_root_profile_but_rejects_assets() {
        assert!(is_allowed_personal_path(&[
            "vault",
            "personal",
            "profile.md"
        ]));
        assert!(!is_allowed_personal_path(&[
            "vault",
            "personal",
            "assets",
            "evidence.png"
        ]));
        assert!(is_allowed_personal_path(&[
            "vault",
            "personal",
            "projects",
            "sounds-from-home",
            "index.md"
        ]));
        assert!(is_allowed_personal_path(&[
            "vault", "personal", "journal", "index.md"
        ]));
    }

    #[test]
    fn harness_curation_categories_are_valid_profile_and_company_paths() {
        assert!(is_allowed_profile_path(&[
            "vault", "profile", "facts", "item.md"
        ]));
        assert!(is_allowed_work_path(&[
            "vault",
            "work",
            "cluml",
            "decisions",
            "item.md"
        ]));
    }

    #[test]
    fn work_layout_rejects_company_assets() {
        assert!(!is_allowed_work_path(&[
            "vault",
            "work",
            "tmaxcloud",
            "assets",
            "evidence.png"
        ]));
        assert!(is_allowed_work_path(&[
            "vault",
            "work",
            "tmaxcloud",
            "experience",
            "no-code-platform",
            "index.md"
        ]));
    }

    #[test]
    fn attachment_must_be_directly_inside_document_bundle() {
        assert_eq!(
            document_bundle_index_path(
                "vault/personal/projects/sounds-from-home/attachments/screen-00.png"
            )
            .as_deref(),
            Some("vault/personal/projects/sounds-from-home/index.md")
        );
        assert!(document_bundle_index_path("vault/personal/assets/screen-00.png").is_none());
        assert!(
            document_bundle_index_path(
                "vault/personal/projects/sounds-from-home/attachments/nested/screen-00.png"
            )
            .is_none()
        );
    }

    #[test]
    fn markdown_asset_links_are_normalized_before_extension_checks() {
        assert_eq!(
            markdown_link_path("attachments/screen-00.png?download=1#preview"),
            "attachments/screen-00.png"
        );
        assert!(is_image_path(markdown_link_path(
            "https://prod-files-secure.s3.us-west-2.amazonaws.com/bucket/image.PNG?x=1"
        )));
        assert!(!is_image_path(markdown_link_path(
            "https://prod-files-secure.s3.us-west-2.amazonaws.com/bucket/CreateObjectSchemaService.html"
        )));
    }

    #[test]
    fn readme_harness_contract_requires_each_semantic_unit_independently() {
        let temp = TempDirectory::new("verification-readme-harness-required");

        for &omitted_unit in README_HARNESS_REQUIRED_UNITS {
            let fixture = README_HARNESS_REQUIRED_UNITS
                .iter()
                .copied()
                .filter(|unit| *unit != omitted_unit)
                .collect::<Vec<_>>()
                .join("\n");
            fs::write(temp.path().join("README.md"), fixture)
                .expect("README contract fixture must be written");

            let mut failures = Failures::default();
            assert_readme_harness_contract(temp.path(), &mut failures);
            assert_eq!(
                failures.messages,
                vec![format!(
                    "README active Harness contract is missing required semantic unit: {omitted_unit}"
                )]
            );
        }
    }

    #[test]
    fn readme_harness_contract_rejects_each_stale_phrase_independently() {
        let temp = TempDirectory::new("verification-readme-harness-stale");
        let valid_contract = README_HARNESS_REQUIRED_UNITS.join("\n");

        for &stale_phrase in README_STALE_HARNESS_PHRASES {
            fs::write(
                temp.path().join("README.md"),
                format!("{valid_contract}\n{stale_phrase}\n"),
            )
            .expect("stale README contract fixture must be written");

            let mut failures = Failures::default();
            assert_readme_harness_contract(temp.path(), &mut failures);
            assert_eq!(
                failures.messages,
                vec![format!(
                    "README contains stale Harness contract phrase: {stale_phrase}"
                )]
            );
        }

        fs::write(
            temp.path().join("README.md"),
            format!("{valid_contract}\n{README_DUPLICATED_COMPANY_ROUTING_PHRASE}\n"),
        )
        .expect("duplicated routing README fixture must be written");
        let mut failures = Failures::default();
        assert_readme_harness_contract(temp.path(), &mut failures);
        assert_eq!(
            failures.messages,
            vec![format!(
                "README duplicates company-specific routing instead of using the canonical scope pointer: {README_DUPLICATED_COMPANY_ROUTING_PHRASE}"
            )]
        );
    }

    #[test]
    fn harness_operating_plan_rejects_duplicated_mutable_status() {
        let temp = TempDirectory::new("verification-harness-status");
        let directory = temp.path().join("vault/profile/preferences");
        fs::create_dir_all(&directory).expect("status fixture directory must be created");
        let path = directory.join("agent-harness-operating-plan.md");
        fs::write(&path, "# Missing status\n").expect("missing status fixture must be written");

        let mut failures = Failures::default();
        assert_harness_operating_contract(temp.path(), &mut failures);
        assert_eq!(
            failures.messages,
            vec![
                "Missing required frontmatter in vault/profile/preferences/agent-harness-operating-plan.md"
            ]
        );

        for (fixture, expected_message) in [
            (
                "---\ntype: purpose\nstatus: draft\n---\n\n호환 계층이나 전환 registry를 만들지 않는다.\n",
                "Harness operating plan in vault/profile/preferences/agent-harness-operating-plan.md must not duplicate mutable `status` state, got `draft`".to_owned(),
            ),
            (
                "---\ntype: purpose\nactivation: active\n---\n\n호환 계층이나 전환 registry를 만들지 않는다.\n",
                "Harness operating plan in vault/profile/preferences/agent-harness-operating-plan.md must not duplicate mutable `activation` state, got `active`".to_owned(),
            ),
            (
                "---\ntype: purpose\nplan_version: 9\n---\n\n호환 계층이나 전환 registry를 만들지 않는다.\n",
                "Harness operating plan in vault/profile/preferences/agent-harness-operating-plan.md must not duplicate mutable `plan_version` state, got `9`".to_owned(),
            ),
            (
                "---\ntype: purpose\nmax_planned_roles: 4\n---\n\n호환 계층이나 전환 registry를 만들지 않는다.\n",
                "Harness operating plan in vault/profile/preferences/agent-harness-operating-plan.md must not duplicate mutable `max_planned_roles` state, got `4`".to_owned(),
            ),
            (
                "---\ntype: rule\n---\n\n호환 계층이나 전환 registry를 만들지 않는다.\n",
                "Harness operating plan in vault/profile/preferences/agent-harness-operating-plan.md must remain a `purpose` document, got Some(\"rule\")".to_owned(),
            ),
        ] {
            fs::write(&path, fixture).expect("invalid status fixture must be written");

            let mut failures = Failures::default();
            assert_harness_operating_contract(temp.path(), &mut failures);
            assert_eq!(failures.messages, vec![expected_message]);
        }

        fs::write(
            &path,
            "---\ntype: purpose\n---\n\n호환 계층이나 전환 registry를 만들지 않는다.\n",
        )
        .expect("valid status fixture must be written");
        let mut failures = Failures::default();
        assert_harness_operating_contract(temp.path(), &mut failures);
        assert!(failures.messages.is_empty());
    }

    #[test]
    fn professional_profile_adapter_guidance_is_pointer_only() {
        let canonical_boundary =
            "../vault/profile/rules/agent-harness.md#professional-profile-boundary";
        let valid_section =
            format!("# Adapter\n\n## Professional profile boundary\n\n{canonical_boundary}\n");
        let mut failures = Failures::default();
        assert_professional_profile_adapter_pointer(
            &format!("{valid_section}\n## LinkedIn integration\n\nLinkedIn mapping.\n"),
            &mut failures,
        );
        assert!(failures.messages.is_empty());

        for duplicated_policy in [
            "ProfessionalProfileArtifact",
            "LinkedIn",
            "Wanted",
            "Remember",
            "external publishing",
            "`Improvement` correction",
        ] {
            let mut failures = Failures::default();
            assert_professional_profile_adapter_pointer(
                &format!("{valid_section}\n{duplicated_policy}\n"),
                &mut failures,
            );
            assert_eq!(
                failures.messages,
                vec![format!(
                    "Harness adapter contract duplicates canonical professional-profile policy: {duplicated_policy}"
                )]
            );
        }

        let mut failures = Failures::default();
        assert_professional_profile_adapter_pointer(
            &format!(
                "# Adapter\n\n{canonical_boundary}\n\n## Professional profile boundary\n\nCore outputs only.\n"
            ),
            &mut failures,
        );
        assert_eq!(
            failures.messages,
            vec![format!(
                "Professional-profile adapter section must point to the canonical boundary: {canonical_boundary}"
            )]
        );

        let mut failures = Failures::default();
        assert_professional_profile_adapter_pointer("# Adapter\n", &mut failures);
        assert_eq!(
            failures.messages,
            vec![
                "Harness adapter contract is missing the Professional profile boundary section"
                    .to_owned()
            ]
        );
    }

    #[test]
    fn harness_architecture_gate_rejects_versioned_commands() {
        let mut failures = Failures::default();
        assert_harness_runtime_source(
            "crates/context-core/src/harness/request.rs",
            "fn dispatch(command: &str) { match command { \"prepare-v12\" => run(), _ => stop() } }",
            &mut failures,
        );
        assert_eq!(failures.messages.len(), 1);
        assert!(failures.messages[0].contains("versioned Harness command"));
    }

    #[test]
    fn harness_architecture_gate_rejects_unversioned_compatibility_commands() {
        let mut failures = Failures::default();
        assert_harness_runtime_source(
            "src/native_harness.rs",
            "fn run(command: &str) { match command { \"resolve-old\" => run(), _ => stop() } }",
            &mut failures,
        );
        assert!(
            failures
                .messages
                .iter()
                .any(|message| message.contains("compatibility Harness command"))
        );
    }

    #[test]
    fn harness_architecture_gate_rejects_any_extra_command() {
        use std::fmt::Write as _;

        let mut arms = String::new();
        for command in CURRENT_HARNESS_COMMANDS {
            write!(arms, "\"{command}\" => run(),").unwrap();
        }
        arms.push_str("\"resolve2\" => run(), _ => stop()");
        let source = format!("fn run(command: &str) {{ match command {{ {arms} }} }}");
        let mut failures = Failures::default();
        assert_harness_runtime_source("src/native_harness.rs", &source, &mut failures);
        assert!(
            failures
                .messages
                .iter()
                .any(|message| message.contains("command set must be exactly"))
        );
    }

    #[test]
    fn harness_architecture_gate_rejects_bridge_features() {
        let mut failures = Failures::default();
        assert_harness_runtime_source(
            "crates/context-core/src/harness/request.rs",
            "mod legacy_bridge; #[serde(alias = \"old\")] struct Current;",
            &mut failures,
        );
        assert!(
            failures
                .messages
                .iter()
                .any(|message| message.contains("compatibility identifier"))
        );
        assert!(
            failures
                .messages
                .iter()
                .any(|message| message.contains("serde `alias`"))
        );
    }

    #[test]
    fn harness_architecture_gate_rejects_parallel_schema_types() {
        let mut failures = Failures::default();
        assert_harness_runtime_source(
            "crates/context-core/src/harness/request.rs",
            "pub(crate) struct ToolExecutionPlanV2;",
            &mut failures,
        );
        assert_eq!(failures.messages.len(), 1);
        assert!(failures.messages[0].contains("version-suffixed"));
    }

    #[test]
    fn harness_architecture_gate_rejects_independent_version_axes() {
        let mut failures = Failures::default();
        assert_harness_runtime_source(
            "crates/context-core/src/harness/request.rs",
            "const TOOL_PLAN_SCHEMA_VERSION: u32 = 2; struct Record { schema_version: u32 }",
            &mut failures,
        );
        assert!(
            failures
                .messages
                .iter()
                .any(|message| message.contains("independent version constant"))
        );
        assert!(
            failures
                .messages
                .iter()
                .any(|message| message.contains("parallel `schema_version`"))
        );
    }

    #[test]
    fn harness_architecture_gate_rejects_relaxed_deserializers() {
        let mut failures = Failures::default();
        assert_harness_runtime_source(
            "crates/context-core/src/harness/request.rs",
            "#[serde(flatten, deserialize_with = \"decode_old\")] struct Current { value: String }",
            &mut failures,
        );
        assert!(
            failures
                .messages
                .iter()
                .any(|message| message.contains("serde `flatten`"))
        );
        assert!(
            failures
                .messages
                .iter()
                .any(|message| message.contains("serde `deserialize_with`"))
        );
    }

    #[test]
    fn harness_architecture_gate_rejects_direct_decoder_bypass() {
        let mut failures = Failures::default();
        assert_harness_runtime_source(
            "crates/context-core/src/harness/request.rs",
            "fn read_old(bytes: &[u8]) { let _: Current = serde_json::from_slice(bytes).unwrap(); }",
            &mut failures,
        );
        assert!(
            failures
                .messages
                .iter()
                .any(|message| message.contains("bypasses decode_current_json"))
        );
    }
}
