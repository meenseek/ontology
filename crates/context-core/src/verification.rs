use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read as _,
    path::Path,
};

use syn::{
    Attribute, Expr, Field, ImplItemFn, ItemConst, ItemEnum, ItemFn, ItemImpl, ItemMod, ItemStruct,
    ItemType, ItemUnion, LitStr, Meta, TypePath,
    visit::{self, Visit},
};

use crate::{ContextError, Result, harness::verified_repository_file};

const CURRENT_HARNESS_RUNTIME_FILES: &[&str] = &[
    "crates/context-core/src/harness.rs",
    "crates/context-core/src/harness/execution.rs",
    "crates/context-core/src/harness/finalization.rs",
    "crates/context-core/src/harness/persistence.rs",
    "crates/context-core/src/harness/policy.rs",
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
        Err(ContextError::invalid_input(message))
    }
}

/// Verify the executable current-contract architecture and safe source inventory.
/// Caller-owned material layouts and migration inventories are outside this gate.
pub fn verify_native_harness_architecture(repo_root: impl AsRef<Path>) -> Result<()> {
    let mut failures = Failures::default();
    assert_native_harness_architecture(repo_root.as_ref(), &mut failures);
    failures.finish("Native Harness architecture verification failed")
}

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
                ContextError::invalid_input(format!("invalid Rust in {path}: {error}"))
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
                audit.failures.push(
                    "Native main must dispatch exactly once to ontology::native_harness::run",
                );
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
                "name=\"ontology\"",
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
            "policy",
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
            if segments == ["ontology", "native_harness", "run"] {
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

fn read_verified_text(repo_root: &Path, relative_path: &str) -> Result<String> {
    let path = repo_root.join(relative_path);
    let mut file =
        verified_repository_file(repo_root, Path::new(relative_path)).map_err(|error| {
            ContextError::invalid_input(format!(
                "failed to safely open `{}`: {error}",
                path.display()
            ))
        })?;
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|source| ContextError::io("read file", path, source))?;
    Ok(text)
}

fn relative_to_root(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map_or_else(|_| to_posix(path), to_posix)
}

fn to_posix(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
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
                "\npub const HARNESS_SCHEMA_VERSION: u32 = 9;\n",
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
                "ontology::native_harness::run",
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

    #[cfg(unix)]
    #[test]
    fn native_architecture_inventory_rejects_linked_sources() {
        use std::os::unix::fs::symlink;
        for hard_link in [false, true] {
            let fixture = native_architecture_fixture();
            let source = fixture.path().join("src/native_harness.rs");
            let other = fixture.path().join("linked-source.rs");
            fs::rename(&source, &other).expect("move owned source");
            if hard_link {
                fs::hard_link(&other, &source).expect("create owned hard link");
            } else {
                symlink(&other, &source).expect("create owned symlink");
            }
            let error = verify_native_harness_architecture(fixture.path())
                .expect_err("linked source must not satisfy source inventory");
            assert!(
                error.to_string().contains("failed to safely open"),
                "{error}"
            );
        }
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
