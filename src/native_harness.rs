use crate::{
    native_context::{NativeContextSession, NativeContextSource},
    native_role::{
        RoleObservations, execute_codex_frontier, validate_codex_binary,
        validate_codex_producer_frontier,
    },
    store::Store,
};
use context_core::harness::{
    CareerCompositionManifest, CareerExecutionReviewReceipt, ComposedRequest, HarnessEngine,
    HarnessError, HarnessExecutionEvent, HarnessExecutionRecord, HarnessExecutionState,
    HarnessPlan, HarnessRouter, HarnessRuntimeCapabilities, PolicyConfiguration,
    PreparedHarnessRun, RequestEnvelope, RoleExecutionResult, ToolExecutionEvidence,
    ToolExecutionPlan, ValidatedRequest, decode_current_json,
};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
pub const MAX_HARNESS_JSON_BYTES: u64 = 32 * 1024 * 1024;
#[derive(Debug)]
pub struct NativeHarnessError {
    message: String,
    role_submission: Option<Box<RoleSubmission>>,
}

#[derive(Debug, serde::Serialize)]
struct RoleSubmission {
    run_identifier: String,
    submitted_result: RoleExecutionResult,
}

#[derive(serde::Serialize)]
struct HarnessDiagnostic<'a> {
    error: &'a str,
}

#[derive(serde::Serialize)]
struct RoleSubmissionDiagnostic<'a> {
    error: &'a str,
    kind: &'static str,
    submission: &'a RoleSubmission,
}
pub type NativeHarnessResult<T> = std::result::Result<T, NativeHarnessError>;
type Result<T> = NativeHarnessResult<T>;
impl NativeHarnessError {
    pub(crate) fn invalid_input(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            role_submission: None,
        }
    }
    pub(crate) fn io(action: &str, _path: impl AsRef<Path>, _error: std::io::Error) -> Self {
        Self::invalid_input(format!("{action} failed"))
    }
    pub(crate) fn submission_error(
        error: HarnessError,
        run_identifier: &str,
        submitted_result: RoleExecutionResult,
    ) -> Self {
        Self {
            message: error.to_string(),
            role_submission: Some(Box::new(RoleSubmission {
                run_identifier: run_identifier.to_owned(),
                submitted_result,
            })),
        }
    }
    pub fn diagnostic_json(&self) -> String {
        if let Some(submission) = self.role_submission.as_deref() {
            serde_json::to_string(&RoleSubmissionDiagnostic {
                error: &self.message,
                kind: "role-submission-error",
                submission,
            })
        } else {
            serde_json::to_string(&HarnessDiagnostic {
                error: &self.message,
            })
        }
        .expect("serialize typed Harness diagnostic")
    }
}
impl std::fmt::Display for NativeHarnessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for NativeHarnessError {}

struct Command {
    source: Arc<NativeContextSource>,
    policy_configuration: PolicyConfiguration,
}
impl Command {
    fn run_harness_command(
        &self,
        mut arguments: impl Iterator<Item = String>,
        observations: &RoleObservations,
    ) -> Result<String> {
        let command = arguments
            .next()
            .ok_or_else(|| NativeHarnessError::invalid_input("missing harness command"))?;
        match command.as_str() {
            "resolve" => self.run_resolve_harness_command(arguments),
            "prepare" => self.run_prepare_harness_command(arguments),
            "replay" => self.run_replay_harness_command(arguments),
            "begin" => self.run_begin_harness_command(arguments, observations),
            "advance" => self.run_advance_harness_command(arguments, observations),
            "revise" => self.run_revise_harness_command(arguments),
            "evaluate" => self.run_evaluate_harness_command(arguments),
            "validate" => self.run_validate_harness_command(arguments),
            "attest-career" => self.run_attest_career_harness_command(arguments),
            "compose-career" => self.run_compose_career_command(arguments),
            _ => Err(NativeHarnessError::invalid_input(format!(
                "unsupported harness command `{command}`"
            ))),
        }
    }

    fn run_resolve_harness_command(
        &self,
        arguments: impl Iterator<Item = String>,
    ) -> Result<String> {
        let input = preflight_resolve_input(
            harness_input(arguments, "resolve")?,
            &self.policy_configuration,
        )?;
        self.run_resolve_harness_input(input)
    }

    fn run_resolve_harness_input(&self, input: HarnessCliInput) -> Result<String> {
        let execution_profile = input
            .composed_request
            .as_ref()
            .map(|r| r.contract().execution_profile())
            .or_else(|| {
                input
                    .validated_request
                    .as_ref()
                    .map(|r| r.contract().execution_profile())
            })
            .ok_or_else(|| {
                NativeHarnessError::invalid_input("resolve input was not preflighted")
            })?;
        let router = HarnessRouter::with_execution_profile(2, execution_profile)
            .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
        let engine = HarnessEngine::with_source_and_router(
            &input.context_view,
            &input.workspace_root,
            self.source.clone(),
            router,
            self.policy_configuration.clone(),
        )
        .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
        let resolved = if let Some(composed) = input.composed_request {
            engine.resolve_composed(composed, input.career_composition_manifest.as_ref())
        } else {
            engine.resolve_validated(
                input.validated_request.ok_or_else(|| {
                    NativeHarnessError::invalid_input("missing validated request")
                })?,
                input.career_composition_manifest.as_ref(),
            )
        }
        .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
        let capabilities_path = input.runtime_capabilities_path.as_deref().ok_or_else(|| {
            NativeHarnessError::invalid_input("resolve requires `--runtime-capabilities`")
        })?;
        let capabilities: HarnessRuntimeCapabilities =
            read_harness_json(capabilities_path, "Harness runtime capabilities")?;
        let plan = HarnessPlan::from_resolved(
            resolved,
            &input.workspace_root,
            capabilities.lifecycle.clone(),
        )
        .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
        plan.validate_capabilities(&capabilities)
            .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
        serialize_json(&plan, "Harness plan")
    }

    #[allow(
        clippy::too_many_lines,
        reason = "Harness preparation keeps strict option parsing and raw-byte binding in one auditable flow"
    )]
    fn run_prepare_harness_command(
        &self,
        arguments: impl IntoIterator<Item = String>,
    ) -> Result<String> {
        let mut context_view = None;
        let mut workspace_root = None;
        let mut plan_path = None;
        let mut capabilities_path = None;
        let mut tool_plan_path = None;
        let mut prepared_output = None;
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            let target = match argument.as_str() {
                "--context-view" => &mut context_view,
                "--workspace-root" => &mut workspace_root,
                "--plan" => &mut plan_path,
                "--runtime-capabilities" => &mut capabilities_path,
                "--tool-plan" => &mut tool_plan_path,
                "--prepared-output" => &mut prepared_output,
                "--json" => continue,
                _ => {
                    return Err(NativeHarnessError::invalid_input(format!(
                        "unexpected Harness prepare argument `{argument}`"
                    )));
                }
            };
            set_harness_option(
                target,
                PathBuf::from(required_inline_value(&mut arguments, &argument)?),
                &argument,
            )?;
        }
        let plan_path =
            plan_path.ok_or_else(|| NativeHarnessError::invalid_input("missing `--plan`"))?;
        let capabilities_path = capabilities_path
            .ok_or_else(|| NativeHarnessError::invalid_input("missing `--runtime-capabilities`"))?;
        let raw_plan = read_harness_bytes(&plan_path, "Harness plan")?;
        let plan: HarnessPlan = decode_current_json(&raw_plan, "Harness plan")
            .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
        let engine = self.harness_engine(
            context_view
                .as_deref()
                .ok_or_else(|| NativeHarnessError::invalid_input("missing `--context-view`"))?,
            workspace_root
                .as_deref()
                .ok_or_else(|| NativeHarnessError::invalid_input("missing `--workspace-root`"))?,
            &plan,
        )?;
        let capabilities: HarnessRuntimeCapabilities =
            read_harness_json(&capabilities_path, "Harness runtime capabilities")?;
        let raw_tool_plan = tool_plan_path
            .as_deref()
            .map(|path| read_harness_bytes(path, "Harness tool plan"))
            .transpose()?;
        let tool_plan = raw_tool_plan
            .as_deref()
            .map(|bytes| {
                decode_current_json::<ToolExecutionPlan>(bytes, "Harness tool plan")
                    .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))
            })
            .transpose()?;
        let prepared = PreparedHarnessRun::prepare(
            &engine,
            &raw_plan,
            plan,
            capabilities,
            raw_tool_plan.as_deref(),
            tool_plan,
        )
        .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
        let prepared_bytes = serde_json::to_vec_pretty(&prepared).map_err(|error| {
            NativeHarnessError::invalid_input(format!("failed to encode prepared run: {error}"))
        })?;
        let prepared_output = prepared_output
            .ok_or_else(|| NativeHarnessError::invalid_input("missing `--prepared-output`"))?;
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&prepared_output)
            .and_then(|mut file| {
                file.write_all(&prepared_bytes)?;
                file.sync_all()
            })
            .map_err(|source| {
                NativeHarnessError::io("write prepared Harness run", &prepared_output, source)
            })?;
        serialize_json(&prepared, "prepared Harness run")
    }

    fn run_replay_harness_command(
        &self,
        arguments: impl IntoIterator<Item = String>,
    ) -> Result<String> {
        let paths = required_harness_paths(
            arguments,
            &["--context-view", "--workspace-root", "--prepared-run"],
            &[],
        )?;
        let (raw, prepared) = read_prepared_run(&paths["--prepared-run"])?;
        let engine = self.harness_engine(
            &paths["--context-view"],
            &paths["--workspace-root"],
            &prepared.plan,
        )?;
        prepared
            .validate_with_engine(&raw, &engine)
            .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
        serialize_json(&prepared, "replayed current Harness run")
    }

    fn run_begin_harness_command(
        &self,
        arguments: impl IntoIterator<Item = String>,
        observations: &RoleObservations,
    ) -> Result<String> {
        let mut context_view = None;
        let mut workspace_root = None;
        let mut prepared_path = None;
        let mut run_identifier = None;
        let mut codex_binary = None;
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--codex-binary" => set_harness_option(
                    &mut codex_binary,
                    PathBuf::from(required_inline_value(&mut arguments, &argument)?),
                    &argument,
                )?,
                "--prepared-run" => set_harness_option(
                    &mut prepared_path,
                    PathBuf::from(required_inline_value(&mut arguments, &argument)?),
                    &argument,
                )?,
                "--run-id" => set_harness_option(
                    &mut run_identifier,
                    required_inline_value(&mut arguments, &argument)?,
                    &argument,
                )?,
                "--context-view" => set_harness_option(
                    &mut context_view,
                    PathBuf::from(required_inline_value(&mut arguments, &argument)?),
                    &argument,
                )?,
                "--workspace-root" => set_harness_option(
                    &mut workspace_root,
                    PathBuf::from(required_inline_value(&mut arguments, &argument)?),
                    &argument,
                )?,
                "--json" => {}
                _ => {
                    return Err(NativeHarnessError::invalid_input(format!(
                        "unexpected Harness begin argument `{argument}`"
                    )));
                }
            }
        }
        let (raw, prepared) = read_prepared_run(
            &prepared_path
                .ok_or_else(|| NativeHarnessError::invalid_input("missing `--prepared-run`"))?,
        )?;
        let engine = self.harness_engine_from_prepared(
            context_view.as_deref(),
            workspace_root.as_deref(),
            &prepared,
        )?;
        let run_identifier = run_identifier
            .ok_or_else(|| NativeHarnessError::invalid_input("missing `--run-id`"))?;
        if let Some(binary) = &codex_binary {
            validate_codex_binary(binary)?;
            validate_codex_producer_frontier(
                &prepared,
                [prepared.plan.resolved_request.plan.primary_producer_role],
            )?;
        }
        let record =
            HarnessExecutionRecord::begin_durable(&engine, &raw, &prepared, &run_identifier)
                .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
        let record = if let Some(binary) = codex_binary {
            execute_codex_frontier(
                &engine,
                &prepared,
                record,
                &binary,
                Some(&self.source),
                observations,
            )
        } else {
            Ok(record)
        };
        record.and_then(|record| serialize_json(&record, "Harness execution record"))
    }

    fn run_advance_harness_command(
        &self,
        arguments: impl IntoIterator<Item = String>,
        observations: &RoleObservations,
    ) -> Result<String> {
        let paths = required_harness_paths(
            arguments,
            &["--context-view", "--workspace-root", "--run-id", "--event"],
            &["--codex-binary"],
        )?;
        let event: HarnessExecutionEvent =
            read_harness_json(&paths["--event"], "Harness execution event")?;
        let (engine, run_identifier) = self.durable_harness_context(&paths)?;
        let codex_prepared = paths
            .get("--codex-binary")
            .map(|binary| {
                validate_codex_binary(binary)?;
                HarnessExecutionRecord::load_durable_prepared(
                    &paths["--workspace-root"],
                    &run_identifier,
                )
                .map(|(_, prepared)| prepared)
                .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))
            })
            .transpose()?;
        let rejected_result = match &event {
            HarnessExecutionEvent::RoleResult { result } => Some((**result).clone()),
            _ => None,
        };
        let next = HarnessExecutionRecord::advance_durable(&engine, &run_identifier, event)
            .map_err(|error| match rejected_result {
                Some(result) => {
                    NativeHarnessError::submission_error(error, &run_identifier, result)
                }
                None => NativeHarnessError::invalid_input(error.to_string()),
            })?;
        let next = match (paths.get("--codex-binary"), codex_prepared.as_ref()) {
            (Some(binary), Some(prepared)) => execute_codex_frontier(
                &engine,
                prepared,
                next,
                binary,
                Some(&self.source),
                observations,
            ),
            _ => Ok(next),
        };
        next.and_then(|next| serialize_json(&next, "advanced Harness execution record"))
    }

    fn run_revise_harness_command(
        &self,
        arguments: impl IntoIterator<Item = String>,
    ) -> Result<String> {
        let paths = required_harness_paths(
            arguments,
            &["--context-view", "--workspace-root", "--run-id"],
            &[],
        )?;
        let (engine, run_identifier) = self.durable_harness_context(&paths)?;
        let next = HarnessExecutionRecord::revise_durable(&engine, &run_identifier)
            .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
        serialize_json(&next, "revised Harness execution record")
    }

    fn run_evaluate_harness_command(
        &self,
        arguments: impl IntoIterator<Item = String>,
    ) -> Result<String> {
        let paths = required_harness_paths(
            arguments,
            &["--context-view", "--workspace-root", "--run-id"],
            &["--tool-evidence"],
        )?;
        let evidence = paths
            .get("--tool-evidence")
            .map(|path| read_harness_json::<ToolExecutionEvidence>(path, "Harness tool evidence"))
            .transpose()?;
        let (engine, run_identifier) = self.durable_harness_context(&paths)?;
        let next =
            HarnessExecutionRecord::evaluate_durable(&engine, &run_identifier, evidence.as_ref())
                .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
        serialize_json(&next, "evaluated Harness execution record")
    }

    fn run_validate_harness_command(
        &self,
        arguments: impl IntoIterator<Item = String>,
    ) -> Result<String> {
        let paths = required_harness_paths(
            arguments,
            &["--context-view", "--workspace-root", "--run-id"],
            &[],
        )?;
        let (engine, run_identifier) = self.durable_harness_context(&paths)?;
        let next = HarnessExecutionRecord::validate_durable(&engine, &run_identifier)
            .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
        serialize_json(&next, "validated Harness execution record")
    }

    fn run_attest_career_harness_command(
        &self,
        arguments: impl IntoIterator<Item = String>,
    ) -> Result<String> {
        let paths = required_harness_paths(
            arguments,
            &[
                "--context-view",
                "--workspace-root",
                "--prepared-run",
                "--execution-record",
            ],
            &[],
        )?;
        let (raw, prepared) = read_prepared_run(&paths["--prepared-run"])?;
        let record: HarnessExecutionRecord = read_harness_json(
            &paths["--execution-record"],
            "validated career review execution record",
        )?;
        let engine = self.harness_engine(
            &paths["--context-view"],
            &paths["--workspace-root"],
            &prepared.plan,
        )?;
        record
            .validate(&prepared, &raw)
            .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
        if record.state != HarnessExecutionState::Validated {
            return Err(NativeHarnessError::invalid_input(
                "career attestation requires a validated non-mutating execution record",
            ));
        }
        let receipt = engine
            .attest_career_execution_review(
                &prepared.plan.resolved_request,
                &prepared.role_run,
                &record.role_execution,
            )
            .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
        serialize_json(&receipt, "career execution review receipt")
    }

    fn durable_harness_context(
        &self,
        paths: &BTreeMap<String, PathBuf>,
    ) -> Result<(HarnessEngine, String)> {
        let run_identifier = paths["--run-id"]
            .to_str()
            .ok_or_else(|| NativeHarnessError::invalid_input("run ID must be valid UTF-8"))?
            .to_owned();
        let (_, prepared) = HarnessExecutionRecord::load_durable_prepared(
            &paths["--workspace-root"],
            &run_identifier,
        )
        .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
        let engine = self.harness_engine(
            &paths["--context-view"],
            &paths["--workspace-root"],
            &prepared.plan,
        )?;
        Ok((engine, run_identifier))
    }

    fn harness_engine_from_prepared(
        &self,
        context_view: Option<&Path>,
        workspace_root: Option<&Path>,
        prepared: &PreparedHarnessRun,
    ) -> Result<HarnessEngine> {
        self.harness_engine(
            context_view
                .ok_or_else(|| NativeHarnessError::invalid_input("missing `--context-view`"))?,
            workspace_root
                .ok_or_else(|| NativeHarnessError::invalid_input("missing `--workspace-root`"))?,
            &prepared.plan,
        )
    }

    fn harness_engine(
        &self,
        context_view: &Path,
        workspace_root: &Path,
        plan: &HarnessPlan,
    ) -> Result<HarnessEngine> {
        let router = HarnessRouter::with_execution_profile(
            plan.resolved_request.plan.max_revisions,
            plan.resolved_request.plan.execution_profile,
        )
        .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
        HarnessEngine::with_source_and_router(
            context_view,
            workspace_root,
            self.source.clone(),
            router,
            self.policy_configuration.clone(),
        )
        .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))
    }

    fn run_compose_career_command(
        &self,
        arguments: impl IntoIterator<Item = String>,
    ) -> Result<String> {
        let mut context_view = None;
        let mut workspace_root = None;
        let mut manifest_path = None;
        let mut holistic_receipt_path = None;
        let mut evidence_receipt_paths = Vec::new();
        let mut output_json = false;
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--context-view" | "--workspace-root" | "--career-manifest"
                | "--holistic-receipt" => {
                    let value = PathBuf::from(required_inline_value(&mut arguments, &argument)?);
                    let destination = match argument.as_str() {
                        "--context-view" => &mut context_view,
                        "--workspace-root" => &mut workspace_root,
                        "--career-manifest" => &mut manifest_path,
                        _ => &mut holistic_receipt_path,
                    };
                    set_harness_option(destination, value, &argument)?;
                }
                "--evidence-receipt" => evidence_receipt_paths.push(PathBuf::from(
                    required_inline_value(&mut arguments, &argument)?,
                )),
                "--json" if !output_json => output_json = true,
                "--json" => {
                    return Err(NativeHarnessError::invalid_input(
                        "duplicate harness option `--json`",
                    ));
                }
                _ => {
                    return Err(NativeHarnessError::invalid_input(format!(
                        "unexpected career composition argument `{argument}`"
                    )));
                }
            }
        }
        let context_view = context_view
            .ok_or_else(|| NativeHarnessError::invalid_input("missing `--context-view`"))?;
        let workspace_root = workspace_root
            .ok_or_else(|| NativeHarnessError::invalid_input("missing `--workspace-root`"))?;
        let manifest: CareerCompositionManifest = read_harness_json(
            &manifest_path
                .ok_or_else(|| NativeHarnessError::invalid_input("missing `--career-manifest`"))?,
            "career composition manifest",
        )?;
        let holistic_review: CareerExecutionReviewReceipt = read_harness_json(
            &holistic_receipt_path
                .ok_or_else(|| NativeHarnessError::invalid_input("missing `--holistic-receipt`"))?,
            "holistic career review receipt",
        )?;
        let evidence_reviews = evidence_receipt_paths
            .iter()
            .map(|path| read_harness_json(path, "owner career review receipt"))
            .collect::<Result<Vec<CareerExecutionReviewReceipt>>>()?;
        let engine = HarnessEngine::with_source(
            context_view,
            workspace_root,
            self.source.clone(),
            self.policy_configuration.clone(),
        )
        .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
        let receipt = engine
            .compose_career_execution_reviews(&manifest, &holistic_review, &evidence_reviews)
            .map_err(|error| harness_command_error(error, output_json))?;
        if output_json {
            serialize_json(&receipt, "career composition receipt")
        } else {
            Ok(format!(
                "경력 산출물 조립 검증 통과 (advisory)\n조립 해시: {}\n완전성은 선언된 manifest와 claim lineage 범위이며 Harness engine가 산출물 의미를 자동 판독하지 않음",
                receipt.composition_digest
            ))
        }
    }
}
fn required_harness_paths(
    arguments: impl IntoIterator<Item = String>,
    required: &[&str],
    optional: &[&str],
) -> Result<BTreeMap<String, PathBuf>> {
    let mut paths = BTreeMap::new();
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        if argument == "--json" {
            continue;
        }
        if !required.contains(&argument.as_str()) && !optional.contains(&argument.as_str()) {
            return Err(NativeHarnessError::invalid_input(format!(
                "unexpected Harness path argument `{argument}`"
            )));
        }
        let path = PathBuf::from(required_inline_value(&mut arguments, &argument)?);
        if paths.insert(argument.clone(), path).is_some() {
            return Err(NativeHarnessError::invalid_input(format!(
                "duplicate Harness option `{argument}`"
            )));
        }
    }
    for flag in required {
        if !paths.contains_key(*flag) {
            return Err(NativeHarnessError::invalid_input(format!(
                "missing `{flag}`"
            )));
        }
    }
    Ok(paths)
}

fn read_prepared_run(path: &Path) -> Result<(Vec<u8>, PreparedHarnessRun)> {
    let raw = read_harness_bytes(path, "strict current prepared run")?;
    let prepared = decode_current_json(&raw, "prepared Harness run")
        .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
    Ok((raw, prepared))
}

fn harness_command_error(error: HarnessError, output_json: bool) -> NativeHarnessError {
    match error {
        HarnessError::ApplyLifecycle { message, receipt } if output_json => {
            let output = serde_json::json!({
                "kind": "apply-lifecycle",
                "message": message,
                "receipt": receipt,
            });
            NativeHarnessError::invalid_input(
                serde_json::to_string_pretty(&output)
                    .unwrap_or_else(|serialization_error| serialization_error.to_string()),
            )
        }
        HarnessError::BatchApply { message, receipt } => {
            let output = serde_json::json!({
                "kind": "batch-apply",
                "message": message,
                "receipt": receipt,
            });
            NativeHarnessError::invalid_input(
                serde_json::to_string_pretty(&output)
                    .unwrap_or_else(|serialization_error| serialization_error.to_string()),
            )
        }
        HarnessError::Finalization { message, applied } => {
            let output = serde_json::json!({
                "kind": "current-post-apply-finalization",
                "message": message,
                "applied": applied,
                "journal": applied.lifecycle_receipt.journal_relative_path,
            });
            NativeHarnessError::invalid_input(
                serde_json::to_string_pretty(&output)
                    .unwrap_or_else(|serialization_error| serialization_error.to_string()),
            )
        }
        other => NativeHarnessError::invalid_input(other.to_string()),
    }
}

#[derive(Debug)]
struct HarnessCliInput {
    context_view: PathBuf,
    workspace_root: PathBuf,
    request_envelope: RequestEnvelope,
    career_composition_manifest: Option<CareerCompositionManifest>,
    runtime_capabilities_path: Option<PathBuf>,
    compose_decisions: bool,
    composed_request: Option<ComposedRequest>,
    validated_request: Option<ValidatedRequest>,
}

fn harness_input(
    arguments: impl IntoIterator<Item = String>,
    _command: &str,
) -> Result<HarnessCliInput> {
    let mut options = HarnessInputOptions::default();
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        if is_disallowed_inline_request_flag(&argument) {
            return Err(NativeHarnessError::invalid_input(format!(
                "inline Harness request flag `{argument}` is not executable; put the resolved meaning and its decision source in `--request-envelope`"
            )));
        }
        if !parse_harness_scope_option(&argument, &mut arguments, &mut options)?
            && !parse_harness_content_option(&argument, &mut arguments, &mut options)?
            && !parse_harness_execution_option(&argument, &mut arguments, &mut options)?
        {
            return Err(NativeHarnessError::invalid_input(format!(
                "unexpected harness argument `{argument}`"
            )));
        }
    }
    options.finish()
}

fn is_disallowed_inline_request_flag(argument: &str) -> bool {
    matches!(
        argument,
        "--action"
            | "--owner"
            | "--company"
            | "--project"
            | "--evidence-company"
            | "--evidence-project"
            | "--evidence-personal-project"
            | "--intent"
            | "--career-surface"
            | "--evidence-source"
            | "--execution-profile"
            | "--target"
            | "--delete-target"
            | "--objective"
            | "--curation-kind"
            | "--confirm-curation"
            | "--curation-source"
            | "--query"
            | "--max-context-bytes"
    )
}

#[derive(Default)]
struct HarnessInputOptions {
    context_view: Option<PathBuf>,
    workspace_root: Option<PathBuf>,
    request_envelope_path: Option<PathBuf>,
    career_manifest_path: Option<PathBuf>,
    runtime_capabilities_path: Option<PathBuf>,
    output_json: bool,
    compose_decisions: bool,
}

impl HarnessInputOptions {
    fn finish(self) -> Result<HarnessCliInput> {
        let request_envelope: RequestEnvelope = read_harness_json(
            &self
                .request_envelope_path
                .ok_or_else(|| NativeHarnessError::invalid_input("missing `--request-envelope`"))?,
            "request envelope",
        )?;
        let validated_request = if self.compose_decisions {
            None
        } else {
            Some(
                request_envelope
                    .validated()
                    .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?,
            )
        };
        let career_composition_manifest = self
            .career_manifest_path
            .as_deref()
            .map(|path| read_harness_json(path, "career composition manifest"))
            .transpose()?;
        Ok(HarnessCliInput {
            context_view: self
                .context_view
                .ok_or_else(|| NativeHarnessError::invalid_input("missing `--context-view`"))?,
            workspace_root: self
                .workspace_root
                .ok_or_else(|| NativeHarnessError::invalid_input("missing `--workspace-root`"))?,
            request_envelope,
            career_composition_manifest,
            runtime_capabilities_path: self.runtime_capabilities_path,
            compose_decisions: self.compose_decisions,
            composed_request: None,
            validated_request,
        })
    }
}

fn parse_harness_scope_option(
    argument: &str,
    arguments: &mut impl Iterator<Item = String>,
    options: &mut HarnessInputOptions,
) -> Result<bool> {
    let handled = match argument {
        "--context-view" => {
            let value = PathBuf::from(required_inline_value(arguments, argument)?);
            set_harness_option(&mut options.context_view, value, argument)?;
            true
        }
        "--workspace-root" => {
            let value = PathBuf::from(required_inline_value(arguments, argument)?);
            set_harness_option(&mut options.workspace_root, value, argument)?;
            true
        }
        "--request-envelope" => {
            let value = PathBuf::from(required_inline_value(arguments, argument)?);
            set_harness_option(&mut options.request_envelope_path, value, argument)?;
            true
        }
        _ => false,
    };
    Ok(handled)
}

fn parse_harness_content_option(
    argument: &str,
    arguments: &mut impl Iterator<Item = String>,
    options: &mut HarnessInputOptions,
) -> Result<bool> {
    match argument {
        "--career-manifest" => {
            let value = PathBuf::from(required_inline_value(arguments, argument)?);
            set_harness_option(&mut options.career_manifest_path, value, argument)?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn parse_harness_execution_option(
    argument: &str,
    arguments: &mut impl Iterator<Item = String>,
    options: &mut HarnessInputOptions,
) -> Result<bool> {
    match argument {
        "--compose-decisions" if !options.compose_decisions => options.compose_decisions = true,
        "--compose-decisions" => {
            return Err(NativeHarnessError::invalid_input(
                "duplicate harness option `--compose-decisions`",
            ));
        }
        "--runtime-capabilities" => {
            let value = PathBuf::from(required_inline_value(arguments, argument)?);
            set_harness_option(&mut options.runtime_capabilities_path, value, argument)?;
        }
        "--json" if !options.output_json => options.output_json = true,
        "--json" => {
            return Err(NativeHarnessError::invalid_input(
                "duplicate harness option `--json`",
            ));
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn read_harness_json<T>(path: &Path, label: &str) -> Result<T>
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let bytes = read_harness_bytes(path, label)?;
    decode_current_json(&bytes, label)
        .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))
}

fn read_harness_bytes(path: &Path, label: &str) -> Result<Vec<u8>> {
    let file = File::open(path).map_err(|error| {
        NativeHarnessError::invalid_input(format!(
            "failed to open {label} `{}`: {error}",
            path.display()
        ))
    })?;
    let metadata = file.metadata().map_err(|error| {
        NativeHarnessError::invalid_input(format!(
            "failed to inspect {label} `{}`: {error}",
            path.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(NativeHarnessError::invalid_input(format!(
            "{label} must be a file no larger than {MAX_HARNESS_JSON_BYTES} bytes"
        )));
    }
    let mut bytes = Vec::new();
    file.take(MAX_HARNESS_JSON_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            NativeHarnessError::invalid_input(format!(
                "failed to read {label} `{}`: {error}",
                path.display()
            ))
        })?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_HARNESS_JSON_BYTES {
        return Err(NativeHarnessError::invalid_input(format!(
            "{label} must contain JSON no larger than {MAX_HARNESS_JSON_BYTES} bytes"
        )));
    }
    Ok(bytes)
}

fn set_harness_option<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<()> {
    if slot.replace(value).is_some() {
        return Err(NativeHarnessError::invalid_input(format!(
            "duplicate harness option `{flag}`"
        )));
    }

    Ok(())
}

fn serialize_json(value: &impl serde::Serialize, description: &str) -> Result<String> {
    let output = serde_json::to_string_pretty(value).map_err(|error| {
        NativeHarnessError::invalid_input(format!("failed to serialize {description}: {error}"))
    })?;
    if output.len() as u64 > MAX_HARNESS_JSON_BYTES {
        return Err(NativeHarnessError::invalid_input(
            "Harness output exceeds the finite limit",
        ));
    }
    Ok(output)
}

fn print_output(output: String) -> Result<()> {
    println!("{output}");
    Ok(())
}

fn required_inline_value(
    arguments: &mut impl Iterator<Item = String>,
    flag: &str,
) -> Result<String> {
    arguments
        .next()
        .ok_or_else(|| NativeHarnessError::invalid_input(format!("missing value for `{flag}`")))
}

/// Parse all options before opening the initialized database. No verb runs migrations.
pub async fn run(arguments: Vec<String>) -> Result<()> {
    let (verb, rest) = arguments
        .split_first()
        .ok_or_else(|| NativeHarnessError::invalid_input("missing harness command"))?;
    let allowed: &[&str] = match verb.as_str() {
        "resolve" => &[
            "--request-envelope",
            "--compose-decisions",
            "--runtime-capabilities",
            "--career-manifest",
        ],
        "prepare" => &[
            "--plan",
            "--runtime-capabilities",
            "--tool-plan",
            "--prepared-output",
        ],
        "replay" => &["--prepared-run"],
        "begin" => &["--prepared-run", "--run-id", "--codex-binary"],
        "advance" => &["--run-id", "--event", "--codex-binary"],
        "revise" | "validate" | "apply" => &["--run-id"],
        "recover" => &["--run-id", "--close-before-apply", "--reason"],
        "evaluate" => &["--run-id", "--tool-evidence"],
        "attest-career" => &["--prepared-run", "--execution-record"],
        "compose-career" => &[
            "--career-manifest",
            "--holistic-receipt",
            "--evidence-receipt",
        ],
        _ => {
            return Err(NativeHarnessError::invalid_input(format!(
                "unsupported harness command `{verb}`"
            )));
        }
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut options = BTreeMap::new();
    let mut forwarded = vec![verb.clone()];
    let mut iter = rest.iter();
    while let Some(flag) = iter.next() {
        if !seen.insert(flag.clone()) && flag != "--evidence-receipt" {
            return Err(NativeHarnessError::invalid_input(format!(
                "duplicate Harness option `{flag}`"
            )));
        }
        if verb == "recover" && flag == "--close-before-apply" {
            options.insert(flag.clone(), String::new());
            continue;
        }
        if flag == "--json" || (verb == "resolve" && flag == "--compose-decisions") {
            forwarded.push(flag.clone());
            continue;
        }
        if !allowed.contains(&flag.as_str())
            && ![
                "--context-view",
                "--store-id",
                "--workspace-root",
                "--policy-config",
            ]
            .contains(&flag.as_str())
        {
            return Err(NativeHarnessError::invalid_input(format!(
                "unexpected Harness path argument `{flag}`"
            )));
        }
        let value = iter
            .next()
            .filter(|v| !v.starts_with("--"))
            .ok_or_else(|| {
                NativeHarnessError::invalid_input(format!("missing value for `{flag}`"))
            })?;
        if !["--store-id", "--policy-config"].contains(&flag.as_str()) {
            forwarded.extend([flag.clone(), value.clone()]);
        }
        options.insert(flag.clone(), value.clone());
    }
    for flag in [
        "--context-view",
        "--store-id",
        "--workspace-root",
        "--policy-config",
    ] {
        if !options.contains_key(flag) {
            return Err(NativeHarnessError::invalid_input(format!(
                "missing `{flag}`"
            )));
        }
    }
    let required: &[&str] = match verb.as_str() {
        "resolve" => &["--request-envelope", "--runtime-capabilities"],
        "prepare" => &["--plan", "--runtime-capabilities", "--prepared-output"],
        "replay" => &["--prepared-run"],
        "begin" => &["--prepared-run", "--run-id"],
        "advance" => &["--run-id", "--event"],
        "attest-career" => &["--prepared-run", "--execution-record"],
        "compose-career" => &["--career-manifest", "--holistic-receipt"],
        _ => &["--run-id"],
    };
    for flag in required {
        if !options.contains_key(*flag) {
            return Err(NativeHarnessError::invalid_input(format!(
                "missing `{flag}`"
            )));
        }
    }
    let policy_configuration = PolicyConfiguration::read(&options["--policy-config"])
        .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
    let close_before_apply = options.contains_key("--close-before-apply");
    if options.contains_key("--reason") != close_before_apply {
        return Err(NativeHarnessError::invalid_input(
            "recover --close-before-apply requires --reason; --reason is otherwise invalid",
        ));
    }
    let resolve_input = if verb == "resolve" {
        Some(preflight_resolve_input(
            harness_input(forwarded.iter().skip(1).cloned(), "resolve")?,
            &policy_configuration,
        )?)
    } else {
        // Closure authenticates exact stored integrity and no effects without
        // replaying a request whose source/configuration may have changed.
        if !close_before_apply {
            preflight_policy_configuration(verb, rest, &options, &policy_configuration)?;
        }
        None
    };
    let view = PathBuf::from(&options["--context-view"]);
    let expected_store = options["--store-id"].clone();
    let url = std::env::var("DATABASE_URL")
        .map_err(|_| NativeHarnessError::invalid_input("DATABASE_URL is required"))?;
    let store =
        Store::for_native(&url).map_err(|e| NativeHarnessError::invalid_input(e.to_string()))?;
    let observations = Arc::new(RoleObservations::default());
    let command_observations = observations.clone();
    let output = if matches!(verb.as_str(), "apply" | "recover") {
        let recover = verb == "recover";
        let workspace = PathBuf::from(&options["--workspace-root"]);
        let run = options
            .get("--run-id")
            .cloned()
            .ok_or_else(|| NativeHarnessError::invalid_input("missing `--run-id`"))?;
        let close_reason = options.get("--reason").cloned();
        store
            .with_native_commit(view, expected_store, move |session| {
                if let Some(reason) = close_reason {
                    let result = HarnessExecutionRecord::close_durable_before_apply(
                        &workspace,
                        &run,
                        &reason,
                        policy_configuration.digest(),
                        session,
                    )
                    .map_err(|e| harness_command_error(e, true))
                    .and_then(|(head, closure)| {
                        serialize_json(
                            &serde_json::json!({"head":head,"pre_apply_close":closure}),
                            "pre-apply Harness closure",
                        )
                    });
                    return Ok(result);
                }
                Ok(apply_or_recover(
                    session,
                    &workspace,
                    &run,
                    recover,
                    policy_configuration,
                ))
            })
            .await
            .map_err(|e| NativeHarnessError::invalid_input(e.to_string()))
    } else {
        store
            .with_native_context(view, move |source| {
                use context_core::harness::ContextSource;
                if source
                    .store_identity()
                    .map_err(|_| crate::domain::Error::Invalid)?
                    .is_none_or(|id| id.store_id != expected_store)
                {
                    return Err(crate::domain::Error::Conflict);
                }
                let command = Command {
                    source,
                    policy_configuration,
                };
                Ok(if let Some(input) = resolve_input {
                    command.run_resolve_harness_input(input)
                } else {
                    command.run_harness_command(forwarded.into_iter(), &command_observations)
                })
            })
            .await
            .map_err(|e| NativeHarnessError::invalid_input(e.to_string()))
    };
    // The closures return only owned output/error values. Their DB connections,
    // source caches and view locks are gone before any potentially blocked sink.
    let result = output.and_then(|result| result).and_then(print_output);
    observations.emit();
    result
}

fn preflight_resolve_input(
    mut input: HarnessCliInput,
    configuration: &PolicyConfiguration,
) -> Result<HarnessCliInput> {
    if input.compose_decisions {
        input.composed_request = Some(
            input
                .request_envelope
                .compose_decisions(configuration)
                .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?,
        );
    } else {
        let validated = input
            .validated_request
            .take()
            .ok_or_else(|| NativeHarnessError::invalid_input("missing strict input validation"))?;
        configuration
            .validate_decision_defaults(validated.decision_trace())
            .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
        configuration
            .validate_contract(validated.contract())
            .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
        input.validated_request = Some(validated);
    }
    Ok(input)
}

fn preflight_policy_configuration(
    verb: &str,
    arguments: &[String],
    options: &BTreeMap<String, String>,
    configuration: &PolicyConfiguration,
) -> Result<()> {
    let check = |resolved: &context_core::harness::ResolvedHarnessRequest| {
        configuration
            .validate_resolved(resolved)
            .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))
    };
    match verb {
        "prepare" => {
            let plan: HarnessPlan =
                read_harness_json(Path::new(&options["--plan"]), "Harness plan")?;
            plan.validate()
                .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
            check(&plan.resolved_request)
        }
        "replay" | "begin" | "attest-career" => {
            let prepared: PreparedHarnessRun = read_harness_json(
                Path::new(&options["--prepared-run"]),
                "prepared Harness run",
            )?;
            prepared
                .plan
                .validate()
                .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
            check(&prepared.plan.resolved_request)
        }
        "compose-career" => {
            for path in std::iter::once(options["--holistic-receipt"].as_str()).chain(
                arguments
                    .windows(2)
                    .filter(|pair| pair[0] == "--evidence-receipt")
                    .map(|pair| pair[1].as_str()),
            ) {
                let receipt: CareerExecutionReviewReceipt =
                    read_harness_json(Path::new(path), "career review receipt")?;
                check(&receipt.resolved)?;
            }
            Ok(())
        }
        "advance" | "revise" | "evaluate" | "validate" | "apply" | "recover" => {
            let (_, prepared) = HarnessExecutionRecord::load_durable_prepared(
                Path::new(&options["--workspace-root"]),
                &options["--run-id"],
            )
            .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
            prepared
                .plan
                .validate()
                .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
            check(&prepared.plan.resolved_request)
        }
        _ => Err(NativeHarnessError::invalid_input(
            "unsupported Harness policy preflight",
        )),
    }
}

fn apply_or_recover(
    session: &mut NativeContextSession,
    workspace: &Path,
    run: &str,
    recover: bool,
    policy_configuration: PolicyConfiguration,
) -> Result<String> {
    use context_core::harness::HarnessApplyAttemptState;
    // Core validates the run locator before the existence probe, which grants no source access.
    let (_, prepared) = HarnessExecutionRecord::load_durable_prepared(workspace, run)
        .map_err(|e| harness_command_error(e, true))?;
    let attempt_path = workspace
        .join(".llm-context-vault-harness/runs")
        .join(run)
        .join("apply-attempt.json");
    let attempt_exists = match fs::symlink_metadata(&attempt_path) {
        Ok(_) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => {
            return Err(NativeHarnessError::invalid_input(
                "Inspect durable apply attempt failed",
            ));
        }
    };
    let result = if recover || attempt_exists {
        // Constructing the provider here would refresh files before Core authenticates recovery.
        let source_connection = session.connection.clone();
        let source_store = session.store.clone();
        let source_handle = session.handle.clone();
        let source_factory = session.recovery_source_factory();
        let handle = HarnessExecutionRecord::open_durable_recovery(workspace, run, Some(session))
            .map_err(|e| harness_command_error(e, true))?;
        let terminal = matches!(
            handle.contract().actual_attempt().attempt_state,
            HarnessApplyAttemptState::AppliedFinalized { .. }
                | HarnessApplyAttemptState::RecoveredFinalized { .. }
                | HarnessApplyAttemptState::AbortedBeforeMutation { .. }
                | HarnessApplyAttemptState::AbortedAfterRollback { .. }
        );
        if !recover && !terminal {
            return Err(NativeHarnessError::invalid_input(
                "Recovery required for the durable apply attempt",
            ));
        }
        let preserved = handle
            .contract()
            .apply()
            .targets()
            .iter()
            .map(|t| t.previous().logical_path.clone())
            .collect();
        let source = source_factory(source_store, source_connection, source_handle, preserved);
        let command = Command {
            source,
            policy_configuration,
        };
        let engine =
            command.harness_engine(command.source.root(), workspace, &handle.prepared().plan)?;
        handle.finish(&engine)
    } else {
        let source = session
            .fresh_source()
            .map_err(|e| harness_command_error(e, true))?;
        let command = Command {
            source,
            policy_configuration,
        };
        let engine = command.harness_engine(command.source.root(), workspace, &prepared.plan)?;
        HarnessExecutionRecord::apply_durable(&engine, run, Some(session))
    }
    .map_err(|e| harness_command_error(e, true))?;
    serialize_json(
        &serde_json::json!({"head":result.0,"apply_attempt":result.1}),
        "finalized Harness apply",
    )
}

#[cfg(test)]
#[path = "../tests/context_fixture.rs"]
mod context_fixture;

#[cfg(test)]
mod tests {
    use std::{
        fmt::Write as _,
        fs,
        path::Path,
        process,
        time::{SystemTime, UNIX_EPOCH},
    };

    use context_core::harness::{
        AnalysisKind, CareerClaimLineage, CareerCoverageMode, CareerOutputSurface,
        CompletedRoleResult, ContextAccess, ContextGrant, ContextGrantPurpose, CurationKind,
        DataOwner, DecisionRecord, DecisionTrace, DraftAnalysisContract, DraftCurationContract,
        DraftOwnedTaskCommon, DraftReviewContract, DraftTaskCommon, DraftTaskRequest, DraftValue,
        DraftWriteContract, HARNESS_SCHEMA_VERSION, HarnessAction, HarnessExecutionEvent,
        HarnessExecutionProfile, HarnessIntent, HarnessRequest, HarnessRole, PolicyDefaultRule,
        ReportedRoleLifecycle, RequestSource, RequirementResult, ResolvedTaskContract,
        ResultEvidenceReference, ReviewKind, RoleExecutionOutcome, RoleExecutionResult,
        RoleLifecycleLimits, RoleRuntimeCapabilities, RoleTaskContract, RoleTerminalState,
        TargetOperation, TargetState, UserConfirmationStatus, UserStatement, WriteKind,
    };
    #[cfg(unix)]
    use context_core::harness::{
        ToolCheck, ToolCheckEvidence, ToolCommandEvidence, ToolEvidenceSet, ToolInvocationContract,
        ToolStdin, ToolTermination, VerificationOwner,
    };
    use sha2::{Digest as _, Sha256};

    use super::*;
    use crate::native_role::*;
    use context_core::harness::{ContextSource, RoleInvocationContract};

    #[test]
    fn codex_promotion_intent_schema_supports_current_career_surfaces() {
        let schema: serde_json::Value =
            serde_json::from_str(codex_output_schema()).expect("native schema must parse");
        let surfaces = schema["$defs"]["intent"]["anyOf"][1]["properties"]["surface"]["enum"]
            .as_array()
            .expect("promotion source intent must declare career surfaces");
        let current = [
            CareerOutputSurface::General,
            CareerOutputSurface::Resume,
            CareerOutputSurface::CareerDescription,
            CareerOutputSurface::Portfolio,
            CareerOutputSurface::ProfessionalProfile,
            CareerOutputSurface::ApplicationEssay,
            CareerOutputSurface::Interview,
        ];
        assert_eq!(surfaces.len(), current.len());
        for surface in current {
            let wire = serde_json::to_value(surface).expect("Core surface must serialize");
            assert!(
                surfaces.contains(&wire),
                "native schema omitted {surface:?}"
            );
            for kind in ["career-artifact", "output-adapter"] {
                let intent = serde_json::json!({"kind": kind, "surface": wire});
                serde_json::from_value::<HarnessIntent>(intent)
                    .expect("native source intent must decode into current Core");
            }
        }
    }

    pub(crate) struct TempDirectory {
        path: PathBuf,
    }

    impl TempDirectory {
        pub(crate) fn new(name: &str) -> Self {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after the Unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "ontology-harness-command-{name}-{}-{timestamp}",
                process::id()
            ));
            fs::create_dir_all(&path).expect("temporary directory should be created");
            let path = path.canonicalize().expect("canonical owned test directory");
            Self { path }
        }

        pub(crate) fn path(&self) -> &PathBuf {
            &self.path
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            test_sources()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .retain(|root, _| !root.starts_with(&self.path));
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn harness_input_parses_request_envelope() {
        let temp = TempDirectory::new("harness-request-envelope");
        let envelope_path = write_user_request_envelope(
            &temp,
            "company-review",
            HarnessRequest {
                action: HarnessAction::CodeReview,
                owner: DataOwner::CompanyProject {
                    company: "example-company".to_owned(),
                    project: "example-project".to_owned(),
                },
                targets: vec!["src/lib.rs".to_owned()],
                objective: "review the change".to_owned(),
                curation_kind: None,
                explicit_user_confirmation_reported: false,
                curation_sources: Vec::new(),
                delete_targets: Vec::new(),
            },
            HarnessIntent::General,
            Vec::new(),
            Vec::new(),
            HarnessExecutionProfile::Standard,
        );
        let input = harness_input(
            [
                "--context-view".to_owned(),
                "/vault-repository".to_owned(),
                "--workspace-root".to_owned(),
                "/workspace".to_owned(),
                "--request-envelope".to_owned(),
                envelope_path.to_string_lossy().into_owned(),
                "--json".to_owned(),
            ],
            "resolve",
        )
        .expect("a complete company review request should parse");

        let (contract, _, _) = input
            .request_envelope
            .resolve()
            .expect("request envelope must resolve");
        assert_eq!(contract.action(), HarnessAction::CodeReview);
        assert_eq!(
            contract.owner(),
            &DataOwner::CompanyProject {
                company: "example-company".to_owned(),
                project: "example-project".to_owned()
            }
        );
    }

    fn read_only_harness_capabilities(
        plan: &HarnessPlan,
        lifecycle: RoleLifecycleLimits,
    ) -> HarnessRuntimeCapabilities {
        HarnessRuntimeCapabilities {
            version: HARNESS_SCHEMA_VERSION,
            available_roles: plan.required_roles().into_iter().collect(),
            max_concurrent_roles: plan.resolved_request.plan.required_concurrent_roles,
            separate_contexts: true,
            file_reading: true,
            tool_execution: plan.requires_tool_capability(),
            deterministic_validation: true,
            max_role_bundle_bytes: 8 * 1024 * 1024,
            max_role_invocation_bytes: 16 * 1024 * 1024,
            lifecycle,
        }
    }

    #[test]
    fn harness_cli_resolves_and_prepares_a_read_only_target() {
        let temp = TempDirectory::new("harness-read-only-target");
        let repository_root = temp.path().join("artifact-root");
        fs::create_dir_all(&repository_root).expect("synthetic artifact root");
        fs::write(
            repository_root.join("README.md"),
            "# Synthetic repository document\n",
        )
        .expect("synthetic read target");
        let envelope_path = write_user_request_envelope(
            &temp,
            "read-only-target",
            HarnessRequest {
                action: HarnessAction::DocumentReview,
                owner: DataOwner::Profile,
                targets: vec!["README.md".to_owned()],
                objective: "review the frozen repository document".to_owned(),
                curation_kind: None,
                explicit_user_confirmation_reported: false,
                curation_sources: Vec::new(),
                delete_targets: Vec::new(),
            },
            HarnessIntent::OutputAdapter {
                surface: CareerOutputSurface::General,
            },
            Vec::new(),
            Vec::new(),
            HarnessExecutionProfile::Standard,
        );
        let envelope: RequestEnvelope = read_harness_json(&envelope_path, "test request envelope")
            .expect("read-only request envelope must be readable");
        let router = HarnessRouter::with_execution_profile(2, HarnessExecutionProfile::Standard)
            .expect("standard router must be valid");
        let engine = test_engine(&repository_root, &repository_root, router)
            .expect("repository Harness engine must open");
        let resolved = engine
            .resolve_envelope(&envelope, None)
            .expect("read-only request must resolve through the Harness engine");
        let lifecycle = RoleLifecycleLimits {
            max_role_execution_millis: 60_000,
            max_role_grace_millis: 10_000,
            max_role_close_millis: 5_000,
            max_total_role_millis: 75_000,
        };
        let plan = HarnessPlan::from_resolved(resolved, &repository_root, lifecycle.clone())
            .expect("read-only target must freeze in the current plan");
        let capabilities = read_only_harness_capabilities(&plan, lifecycle);
        let capabilities_path = temp.path().join("capabilities.json");
        write_test_json(&capabilities_path, &capabilities);

        run([
            "ontology".to_owned(),
            "harness".to_owned(),
            "resolve".to_owned(),
            "--context-view".to_owned(),
            repository_root.to_string_lossy().into_owned(),
            "--workspace-root".to_owned(),
            repository_root.to_string_lossy().into_owned(),
            "--request-envelope".to_owned(),
            envelope_path.to_string_lossy().into_owned(),
            "--runtime-capabilities".to_owned(),
            capabilities_path.to_string_lossy().into_owned(),
            "--json".to_owned(),
        ])
        .expect("public resolve command must accept a read-only target");

        let plan_path = temp.path().join("plan.json");
        let prepared_path = temp.path().join("prepared.json");
        write_test_json(&plan_path, &plan);
        run([
            "ontology".to_owned(),
            "harness".to_owned(),
            "prepare".to_owned(),
            "--context-view".to_owned(),
            repository_root.to_string_lossy().into_owned(),
            "--workspace-root".to_owned(),
            repository_root.to_string_lossy().into_owned(),
            "--plan".to_owned(),
            plan_path.to_string_lossy().into_owned(),
            "--runtime-capabilities".to_owned(),
            capabilities_path.to_string_lossy().into_owned(),
            "--prepared-output".to_owned(),
            prepared_path.to_string_lossy().into_owned(),
            "--json".to_owned(),
        ])
        .expect("public prepare command must accept a read-only target plan");

        let prepared: PreparedHarnessRun =
            read_harness_json(&prepared_path, "prepared read-only Harness run")
                .expect("prepared read-only run must be readable");
        assert_eq!(
            prepared.plan.frozen_targets.targets[0].operation,
            TargetOperation::Inspect
        );
        assert!(
            prepared.plan.frozen_targets.targets[0]
                .content_digest
                .is_some()
        );
        assert!(!prepared.plan.source_write_allowed);

        assert_public_replay_succeeds(&repository_root, &prepared_path);
        assert_public_replay_rejects_rehashed_role_input(&repository_root, &temp, &prepared);
    }

    #[test]
    fn harness_input_parses_execution_profile() {
        for (profile, expected) in [
            ("standard", HarnessExecutionProfile::Standard),
            ("strict", HarnessExecutionProfile::Strict),
        ] {
            let temp = TempDirectory::new(profile);
            let envelope_path = write_user_request_envelope(
                &temp,
                "execution-profile",
                HarnessRequest {
                    action: HarnessAction::CodeReview,
                    owner: DataOwner::CompanyProject {
                        company: "example-company".to_owned(),
                        project: "example-project".to_owned(),
                    },
                    targets: vec!["src/lib.rs".to_owned()],
                    objective: "review the change".to_owned(),
                    curation_kind: None,
                    explicit_user_confirmation_reported: false,
                    curation_sources: Vec::new(),
                    delete_targets: Vec::new(),
                },
                HarnessIntent::General,
                Vec::new(),
                Vec::new(),
                expected,
            );
            let input = harness_input(
                [
                    "--context-view".to_owned(),
                    "/vault-repository".to_owned(),
                    "--workspace-root".to_owned(),
                    "/workspace".to_owned(),
                    "--request-envelope".to_owned(),
                    envelope_path.to_string_lossy().into_owned(),
                ],
                "resolve",
            )
            .expect("a complete request with execution profile should parse");

            let (contract, _, _) = input
                .request_envelope
                .resolve()
                .expect("request envelope must resolve");
            assert_eq!(contract.execution_profile(), expected);
        }
    }

    #[test]
    fn harness_input_parses_read_only_company_evidence_grant() {
        let temp = TempDirectory::new("harness-company-evidence");
        let grant = ContextGrant {
            owner: DataOwner::Company {
                company: "cedar".to_owned(),
            },
            purpose: ContextGrantPurpose::CareerWritingEvidence,
            access: ContextAccess::ReadOnly,
        };
        let envelope_path = write_user_request_envelope(
            &temp,
            "company-evidence",
            HarnessRequest {
                action: HarnessAction::Investigation,
                owner: DataOwner::Personal,
                targets: Vec::new(),
                objective: "review career evidence".to_owned(),
                curation_kind: None,
                explicit_user_confirmation_reported: false,
                curation_sources: Vec::new(),
                delete_targets: Vec::new(),
            },
            HarnessIntent::CareerArtifact {
                surface: CareerOutputSurface::General,
            },
            vec![grant.clone()],
            Vec::new(),
            HarnessExecutionProfile::Standard,
        );
        let input = harness_input(
            [
                "--context-view".to_owned(),
                "/vault-repository".to_owned(),
                "--workspace-root".to_owned(),
                "/workspace".to_owned(),
                "--request-envelope".to_owned(),
                envelope_path.to_string_lossy().into_owned(),
            ],
            "resolve",
        )
        .expect("personal investigation with company evidence should parse");

        let (contract, _, _) = input
            .request_envelope
            .resolve()
            .expect("request envelope must resolve");
        assert_eq!(contract.context_grants(), [grant]);
    }

    #[test]
    fn harness_input_parses_career_surface_and_personal_project_evidence() {
        let temp = TempDirectory::new("harness-career-surface");
        let grant = ContextGrant {
            owner: DataOwner::PersonalProject {
                project: "sample".to_owned(),
            },
            purpose: ContextGrantPurpose::CareerWritingEvidence,
            access: ContextAccess::ReadOnly,
        };
        let envelope_path = write_user_request_envelope(
            &temp,
            "career-surface",
            HarnessRequest {
                action: HarnessAction::DocumentReview,
                owner: DataOwner::Personal,
                targets: vec!["docs/portfolio.md".to_owned()],
                objective: "review portfolio evidence".to_owned(),
                curation_kind: None,
                explicit_user_confirmation_reported: false,
                curation_sources: Vec::new(),
                delete_targets: Vec::new(),
            },
            HarnessIntent::CareerArtifact {
                surface: CareerOutputSurface::Portfolio,
            },
            vec![grant.clone()],
            Vec::new(),
            HarnessExecutionProfile::Standard,
        );
        let input = harness_input(
            [
                "--context-view".to_owned(),
                "/vault-repository".to_owned(),
                "--workspace-root".to_owned(),
                "/workspace".to_owned(),
                "--request-envelope".to_owned(),
                envelope_path.to_string_lossy().into_owned(),
            ],
            "resolve",
        )
        .expect("career surface and personal project evidence should parse");

        let (contract, _, _) = input
            .request_envelope
            .resolve()
            .expect("request envelope must resolve");
        assert_eq!(
            contract.intent(),
            HarnessIntent::CareerArtifact {
                surface: CareerOutputSurface::Portfolio,
            }
        );
        assert_eq!(contract.context_grants(), [grant]);
    }

    #[test]
    fn harness_input_rejects_inline_request_flags() {
        let error = harness_input(
            [
                "--context-view".to_owned(),
                "/vault-repository".to_owned(),
                "--workspace-root".to_owned(),
                "/workspace".to_owned(),
                "--action".to_owned(),
                "document-review".to_owned(),
            ],
            "resolve",
        )
        .expect_err("inline request flags must not execute");

        assert!(error.to_string().contains("inline Harness request flag"));
        assert!(error.to_string().contains("--request-envelope"));
    }

    #[test]
    fn harness_rejects_removed_commands_and_inline_submission_flags() {
        let command_error = run_harness_command(["template".to_owned()])
            .expect_err("removed template command must not execute");
        assert!(
            command_error
                .to_string()
                .contains("unsupported harness command `template`")
        );

        let flag_error = harness_input(
            [
                "--context-view".to_owned(),
                "/vault-repository".to_owned(),
                "--workspace-root".to_owned(),
                "/workspace".to_owned(),
                "--submission".to_owned(),
                "/tmp/submission.json".to_owned(),
            ],
            "validate",
        )
        .expect_err("inline submission flag must not execute");
        assert!(
            flag_error
                .to_string()
                .contains("unexpected harness argument")
        );
    }

    #[test]
    fn batch_apply_json_error_preserves_the_recovery_receipt() {
        use context_core::harness::{
            BatchApplyOutcomeReceipt, HarnessBatchApplyLifecycleReceipt,
            LifecycleConfirmationReceipt, WorkspaceMutationLockReceipt,
        };

        for output_json in [false, true] {
            let error = harness_command_error(
                HarnessError::BatchApply {
                    message: "manual recovery required".to_owned(),
                    receipt: Box::new(HarnessBatchApplyLifecycleReceipt {
                        batch_id: "batch-example".to_owned(),
                        resolved_plan_digest: "1".repeat(64),
                        candidate_digest: "2".repeat(64),
                        journal_relative_path:
                            ".llm-context-vault-harness/batches/batch-example/journal.json"
                                .to_owned(),
                        completion_receipt_relative_path: None,
                        journal_retained: true,
                        outcome: BatchApplyOutcomeReceipt::RecoveryRequired,
                        lock: WorkspaceMutationLockReceipt {
                            lock_file_creation_committed: true,
                            lock_content_durability: LifecycleConfirmationReceipt::Confirmed,
                            creation_parent_durability: LifecycleConfirmationReceipt::Confirmed,
                            explicit_unlink_committed: true,
                            verified_final_absence: LifecycleConfirmationReceipt::Confirmed,
                            unlink_parent_durability: LifecycleConfirmationReceipt::Confirmed,
                            failures: Vec::new(),
                        },
                        targets: Vec::new(),
                        failure_history: Vec::new(),
                        orchestration_failures: Vec::new(),
                    }),
                },
                output_json,
            );
            let output = error.to_string();

            assert!(output.contains("\"kind\": \"batch-apply\""));
            assert!(output.contains("\"journal_retained\": true"));
            assert!(output.contains("batch-example/journal.json"));
            assert!(output.contains(&"1".repeat(64)));
            assert!(output.contains(&"2".repeat(64)));
        }
    }

    #[test]
    fn harness_input_rejects_structured_caller_envelopes() {
        let temp = TempDirectory::new("harness-structured-caller");
        let envelope_path = write_user_request_envelope(
            &temp,
            "structured-caller",
            HarnessRequest {
                action: HarnessAction::DocumentReview,
                owner: DataOwner::Personal,
                targets: vec!["docs/guide.md".to_owned()],
                objective: "review the guide".to_owned(),
                curation_kind: None,
                explicit_user_confirmation_reported: false,
                curation_sources: Vec::new(),
                delete_targets: Vec::new(),
            },
            HarnessIntent::General,
            Vec::new(),
            Vec::new(),
            HarnessExecutionProfile::Standard,
        );
        let mut envelope: RequestEnvelope =
            read_harness_json(&envelope_path, "test request envelope")
                .expect("request envelope must be readable");
        envelope.source = RequestSource::StructuredCaller {
            description: "untrusted adapter".to_owned(),
        };
        write_test_json(&envelope_path, &envelope);

        let error = harness_input(
            [
                "--context-view".to_owned(),
                "/vault-repository".to_owned(),
                "--workspace-root".to_owned(),
                "/workspace".to_owned(),
                "--request-envelope".to_owned(),
                envelope_path.to_string_lossy().into_owned(),
            ],
            "resolve",
        )
        .expect_err("structured-caller envelope must not execute");

        assert!(
            error
                .to_string()
                .contains("structured-caller request envelopes are not executable")
        );
    }

    #[test]
    fn harness_content_options_parse_manifest() {
        let mut options = HarnessInputOptions::default();
        let mut manifest_arguments = ["/tmp/career-manifest.json".to_owned()].into_iter();
        assert!(
            parse_harness_content_option(
                "--career-manifest",
                &mut manifest_arguments,
                &mut options,
            )
            .expect("career manifest option must parse")
        );

        assert_eq!(
            options.career_manifest_path,
            Some(PathBuf::from("/tmp/career-manifest.json"))
        );
    }

    #[test]
    fn compose_career_command_requires_owner_receipts() {
        let error = run_harness_command(
            [
                "compose-career",
                "--context-view",
                "/unopened-view",
                "--workspace-root",
                "/unopened-workspace",
                "--store-id",
                "test",
                "--policy-config",
                "crates/context-core/tests/fixtures/policy-settings.json",
                "--career-manifest",
                "/manifest",
                "--holistic-receipt",
                "/holistic",
            ]
            .map(str::to_owned),
        )
        .expect_err("nonempty foreign owners still require matching receipts later");

        assert!(!error.to_string().contains("missing `--evidence-receipt`"));
    }

    #[test]
    fn production_binary_exposes_only_unversioned_harness_commands() {
        for command in [
            "resolve-v8",
            "resolve-v9",
            "prepare-current",
            "recover-apply",
            "inspect-record-version",
            "status",
            "activate-v9",
        ] {
            let error = run([
                "ontology".to_owned(),
                "harness".to_owned(),
                command.to_owned(),
            ])
            .expect_err("versioned or removed Harness commands must be absent");
            assert!(error.to_string().contains("unsupported harness command"));
        }

        for command in [
            "prepare", "begin", "advance", "validate", "apply", "recover",
        ] {
            let error = run([
                "ontology".to_owned(),
                "harness".to_owned(),
                command.to_owned(),
            ])
            .expect_err("unversioned commands without required paths must reach their parser");
            assert!(error.to_string().contains("missing `--"));
            assert!(!error.to_string().contains("unsupported harness command"));
        }
    }

    #[test]
    fn durable_execution_commands_reject_record_paths_and_caller_recovery_journals() {
        for (command, removed_flag) in [
            ("advance", "--execution-record"),
            ("revise", "--prepared-run"),
            ("evaluate", "--execution-record"),
            ("validate", "--prepared-run"),
            ("apply", "--execution-record"),
            ("recover", "--journal"),
        ] {
            let error = run_harness_command([
                command.to_owned(),
                "--context-view".to_owned(),
                "/vault".to_owned(),
                "--workspace-root".to_owned(),
                "/workspace".to_owned(),
                "--run-id".to_owned(),
                "run-current".to_owned(),
                removed_flag.to_owned(),
                "/tmp/removed.json".to_owned(),
            ])
            .expect_err("removed execution-file flags must fail at the command parser");
            assert!(
                error
                    .to_string()
                    .contains("unexpected Harness path argument"),
                "{command} must reject {removed_flag}: {error}"
            );
        }
    }

    fn run_test_compose_career_command(
        repository_root: &Path,
        workspace: &Path,
        manifest_path: &Path,
        holistic_receipt_path: &Path,
        owner_receipt_path: &Path,
    ) {
        run_harness_command([
            "compose-career".to_owned(),
            "--context-view".to_owned(),
            repository_root.to_string_lossy().into_owned(),
            "--workspace-root".to_owned(),
            workspace.to_string_lossy().into_owned(),
            "--career-manifest".to_owned(),
            manifest_path.to_string_lossy().into_owned(),
            "--holistic-receipt".to_owned(),
            holistic_receipt_path.to_string_lossy().into_owned(),
            "--evidence-receipt".to_owned(),
            owner_receipt_path.to_string_lossy().into_owned(),
            "--json".to_owned(),
        ])
        .expect("compose-career CLI must read attest-career receipt JSON successfully");
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "career CLI smoke scenario and receipt digest ordering remain auditable together"
    )]
    fn career_review_receipt_schema_feeds_compose_command_successfully() {
        let temp = TempDirectory::new("career-cli-smoke");
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&workspace).expect("career workspace must be created");
        fs::write(workspace.join("README.md"), "# Career artifact\n")
            .expect("career artifact must be written");
        let repository_root = temp.path().join("artifact-root");
        fs::create_dir_all(&repository_root).expect("synthetic artifact root");
        fs::write(
            repository_root.join("README.md"),
            "# Synthetic repository document\n",
        )
        .expect("synthetic read target");
        let source = "vault/personal/projects/sample.md".to_owned();
        let comparison = context_fixture::career_comparison(
            test_source(&repository_root).as_ref(),
            "review frozen career artifact",
            "sample-career-claim",
            "README.md",
        );
        let manifest = CareerCompositionManifest {
            comparison: Some(comparison),
            version: HARNESS_SCHEMA_VERSION,
            career_output_surface: CareerOutputSurface::Resume,
            artifact_targets: vec!["README.md".to_owned()],
            coverage: CareerCoverageMode::Selected,
            complete_coverage_confirmation_reported: false,
            evidence_owners: vec![DataOwner::PersonalProject {
                project: "sample".to_owned(),
            }],
            canonical_evidence_owners: Vec::new(),
            excluded_evidence_owners: Vec::new(),
            claim_lineage: vec![CareerClaimLineage {
                claim_id: "sample-career-claim".to_owned(),
                evidence_owner: DataOwner::PersonalProject {
                    project: "sample".to_owned(),
                },
                evidence_source_paths: vec![source.clone()],
            }],
        };
        let capabilities = RoleRuntimeCapabilities {
            available_roles: vec![HarnessRole::Verifier, HarnessRole::Reviewer],
            max_concurrent_roles: 2,
            separate_contexts: true,
            file_reading: true,
            tool_execution: true,
            max_role_bundle_bytes: 8 * 1024 * 1024,
            max_role_invocation_bytes: 16 * 1024 * 1024,
            max_role_execution_millis: 60 * 60 * 1_000,
            max_role_grace_millis: 5 * 60 * 1_000,
        };
        let manifest_path = temp.path().join("manifest.json");
        write_test_json(&manifest_path, &manifest);

        let engine = test_engine_default(&repository_root, &workspace)
            .expect("career CLI smoke engine must open");
        let request = HarnessRequest {
            action: HarnessAction::DocumentReview,
            owner: DataOwner::Personal,
            targets: vec!["README.md".to_owned()],
            objective: "review frozen career artifact".to_owned(),
            curation_kind: None,
            explicit_user_confirmation_reported: false,
            curation_sources: Vec::new(),
            delete_targets: Vec::new(),
        };
        let grant = ContextGrant {
            owner: DataOwner::PersonalProject {
                project: "sample".to_owned(),
            },
            purpose: ContextGrantPurpose::CareerWritingEvidence,
            access: ContextAccess::ReadOnly,
        };
        let request_envelope_path = write_user_request_envelope(
            &temp,
            "career-review",
            request.clone(),
            HarnessIntent::CareerArtifact {
                surface: CareerOutputSurface::Resume,
            },
            vec![grant.clone()],
            vec![source.clone()],
            HarnessExecutionProfile::Standard,
        );
        let owner_envelope: RequestEnvelope =
            read_harness_json(&request_envelope_path, "test request envelope")
                .expect("owner request envelope must be readable");
        let owner_receipt =
            test_career_receipt(&engine, &owner_envelope, &manifest, &capabilities, "owner");
        let mut holistic_request = request.clone();
        holistic_request.objective = manifest
            .holistic_task_statement(std::slice::from_ref(&owner_receipt))
            .expect("current owner review handoff");
        let holistic_envelope_path = write_user_request_envelope(
            &temp,
            "holistic-career-review",
            holistic_request,
            HarnessIntent::CareerArtifact {
                surface: CareerOutputSurface::Resume,
            },
            Vec::new(),
            vec!["vault/personal/profile.md".to_owned()],
            HarnessExecutionProfile::Standard,
        );
        let holistic_envelope: RequestEnvelope =
            read_harness_json(&holistic_envelope_path, "test request envelope")
                .expect("holistic request envelope must be readable");
        let holistic_receipt = test_career_receipt(
            &engine,
            &holistic_envelope,
            &manifest,
            &capabilities,
            "holistic",
        );
        let owner_receipt_path = temp.path().join("owner-receipt.json");
        let holistic_receipt_path = temp.path().join("holistic-receipt.json");
        write_test_json(&owner_receipt_path, &owner_receipt);
        write_test_json(&holistic_receipt_path, &holistic_receipt);
        run_test_compose_career_command(
            &repository_root,
            &workspace,
            &manifest_path,
            &holistic_receipt_path,
            &owner_receipt_path,
        );
    }

    #[allow(
        clippy::too_many_lines,
        reason = "career execution scenario and role-result digest ordering remain auditable together"
    )]
    fn test_career_receipt(
        engine: &HarnessEngine,
        request_envelope: &RequestEnvelope,
        manifest: &CareerCompositionManifest,
        capabilities: &RoleRuntimeCapabilities,
        context_suffix: &str,
    ) -> CareerExecutionReviewReceipt {
        let resolved = engine
            .resolve_envelope(request_envelope, Some(manifest))
            .expect("career CLI smoke plan must resolve");
        let prepared = engine
            .prepare(&resolved, capabilities, None)
            .expect("career CLI smoke plan must prepare");
        let mut step = engine
            .begin_execution(&resolved, &prepared, &[])
            .expect("career execution must begin");
        while let Some(invocation) = step.ready_role_invocations.first().cloned() {
            let metadata = prepared
                .role_metadata
                .iter()
                .find(|metadata| metadata.role == invocation.role)
                .expect("invoked role must have prepared metadata");
            let TargetState::Existing { content_digest } = &resolved.plan.targets[0].state else {
                panic!("career review target must exist");
            };
            let evidence = ResultEvidenceReference::Target {
                workspace_relative_path: resolved.plan.targets[0].workspace_relative_path.clone(),
                content_digest: content_digest.clone(),
                locator: "entire artifact".to_owned(),
            };
            let result = match &metadata.task {
                RoleTaskContract::Verifier {
                    verification_requirements,
                    ..
                } => CompletedRoleResult::Verifier {
                    subject_evidence: vec![evidence.clone()],
                    requirement_results: verification_requirements
                        .iter()
                        .map(|requirement| RequirementResult {
                            unit: requirement.unit,
                            passed: true,
                            detail: "verified".to_owned(),
                            evidence: vec![evidence.clone()],
                        })
                        .collect(),
                },
                RoleTaskContract::Reviewer {
                    verification_requirements,
                    ..
                } => CompletedRoleResult::Reviewer {
                    summary: "reviewed".to_owned(),
                    subject_evidence: vec![evidence.clone()],
                    requirement_results: verification_requirements
                        .iter()
                        .map(|requirement| RequirementResult {
                            unit: requirement.unit,
                            passed: true,
                            detail: "reviewed".to_owned(),
                            evidence: vec![evidence.clone()],
                        })
                        .collect(),
                    blocking_findings: Vec::new(),
                    improvements: Vec::new(),
                    learning_candidates: Vec::new(),
                },
                RoleTaskContract::Writer { .. } | RoleTaskContract::Specialist { .. } => {
                    panic!("career document review must use review roles")
                }
            };
            let context_id = format!("{:?}-{context_suffix}", invocation.role).to_lowercase();
            let lifecycle = ReportedRoleLifecycle {
                role: invocation.role,
                context_id: context_id.clone(),
                started_at_millis: 1,
                context_ready_at_millis: 2,
                first_output_at_millis: Some(3),
                interrupt_requested_at_millis: None,
                grace_deadline_at_millis: None,
                terminal_at_millis: 4,
                closed_at_millis: 5,
                terminal_state: RoleTerminalState::Completed,
            };
            let outcome = RoleExecutionOutcome::Completed { result };
            let result_digest = test_serialized_digest(&(
                invocation.role,
                &invocation.invocation_digest,
                &context_id,
                &lifecycle,
                &outcome,
            ));
            step = engine
                .advance_execution(
                    &resolved,
                    &prepared,
                    &[],
                    &step.record,
                    HarnessExecutionEvent::RoleResult {
                        result: Box::new(RoleExecutionResult {
                            role: invocation.role,
                            invocation_digest: invocation.invocation_digest,
                            context_id,
                            lifecycle,
                            outcome,
                            result_digest,
                        }),
                    },
                )
                .expect("career role result must advance");
        }
        engine
            .attest_career_execution_review(&resolved, &prepared, &step.record)
            .expect("career CLI smoke execution review must attest")
    }

    fn write_test_json(path: &Path, value: &impl serde::Serialize) {
        let bytes = serde_json::to_vec_pretty(value).expect("test JSON must serialize");
        fs::write(path, bytes).expect("test JSON must be written");
    }

    fn assert_public_replay_succeeds(repository_root: &Path, prepared_path: &Path) {
        run([
            "ontology".to_owned(),
            "harness".to_owned(),
            "replay".to_owned(),
            "--context-view".to_owned(),
            repository_root.to_string_lossy().into_owned(),
            "--workspace-root".to_owned(),
            repository_root.to_string_lossy().into_owned(),
            "--prepared-run".to_owned(),
            prepared_path.to_string_lossy().into_owned(),
            "--json".to_owned(),
        ])
        .expect("public replay command must revalidate the exact engine preparation");
    }

    fn assert_public_replay_rejects_rehashed_role_input(
        repository_root: &Path,
        temp: &TempDirectory,
        prepared: &PreparedHarnessRun,
    ) {
        let mut forged = prepared.clone();
        let reviewer = forged
            .role_run
            .role_metadata
            .iter_mut()
            .find(|metadata| metadata.role == HarnessRole::Reviewer)
            .expect("prepared document review must contain Reviewer metadata");
        let RoleTaskContract::Reviewer { targets, .. } = &mut reviewer.task else {
            panic!("Reviewer metadata must contain a Reviewer task");
        };
        targets[0].workspace_relative_path = "docs/forged.md".to_owned();
        forged.role_run.prepared_role_run_digest = test_serialized_digest(&(
            &forged.role_run.resolved_plan_digest,
            &forged.role_run.runtime_capabilities_digest,
            &forged.role_run.context_bundle,
            &forged.role_run.role_bundles,
            &forged.role_run.role_metadata,
        ));
        forged.prepared_run_digest = test_serialized_digest(&(
            forged.version,
            &forged.plan_identity,
            &forged.tool_plan_identity,
            &forged.plan,
            &forged.runtime_capabilities,
            &forged.accepted_tool_plan,
            &forged.role_run,
        ));
        let forged_path = temp.path().join("forged-prepared.json");
        write_test_json(&forged_path, &forged);
        assert_public_replay_rejects_role_metadata_mutation(repository_root, &forged_path);
    }

    fn assert_public_replay_rejects_role_metadata_mutation(
        repository_root: &Path,
        prepared_path: &Path,
    ) {
        let error = run([
            "ontology".to_owned(),
            "harness".to_owned(),
            "replay".to_owned(),
            "--context-view".to_owned(),
            repository_root.to_string_lossy().into_owned(),
            "--workspace-root".to_owned(),
            repository_root.to_string_lossy().into_owned(),
            "--prepared-run".to_owned(),
            prepared_path.to_string_lossy().into_owned(),
            "--json".to_owned(),
        ])
        .expect_err("public replay must reject rehashed prepared role metadata");
        assert!(
            error
                .to_string()
                .contains("not the exact current Harness engine preparation")
        );
    }

    #[allow(
        clippy::too_many_lines,
        reason = "request-envelope scenario construction and decision digest ordering remain auditable together"
    )]
    fn write_user_request_envelope(
        temp: &TempDirectory,
        name: &str,
        request: HarnessRequest,
        intent: HarnessIntent,
        context_grants: Vec<ContextGrant>,
        evidence_source_paths: Vec<String>,
        execution_profile: HarnessExecutionProfile,
    ) -> PathBuf {
        let statement_identifier = "statement-0000000000000001".to_owned();
        let mut records = Vec::new();
        let mut next_decision = 1_u64;
        let draft = if request.action == HarnessAction::VaultCuration {
            let curation_kind = request
                .curation_kind
                .expect("curation test request must include a kind");
            let target = request
                .targets
                .first()
                .expect("curation test request must include a target")
                .clone();
            DraftTaskRequest::Curation(Box::new(DraftCurationContract {
                common: DraftOwnedTaskCommon {
                    owner: test_user_value(
                        request.owner,
                        &statement_identifier,
                        &mut records,
                        &mut next_decision,
                    ),
                    task_statement: test_user_value(
                        request.objective.clone(),
                        &statement_identifier,
                        &mut records,
                        &mut next_decision,
                    ),
                    execution_profile: test_execution_profile_value(
                        execution_profile,
                        &statement_identifier,
                        &mut records,
                        &mut next_decision,
                    ),
                },
                curation_kind: test_user_value(
                    curation_kind,
                    &statement_identifier,
                    &mut records,
                    &mut next_decision,
                ),
                target: test_user_value(
                    target,
                    &statement_identifier,
                    &mut records,
                    &mut next_decision,
                ),
                curation_sources: test_optional_user_value(
                    request.curation_sources,
                    &statement_identifier,
                    &mut records,
                    &mut next_decision,
                ),
                delete_target: request.delete_targets.into_iter().next().map(|target| {
                    test_user_value(
                        target,
                        &statement_identifier,
                        &mut records,
                        &mut next_decision,
                    )
                }),
                promotion_handoff: None,
                confirmation: test_confirmation_value(
                    if request.explicit_user_confirmation_reported {
                        UserConfirmationStatus::Confirmed
                    } else {
                        UserConfirmationStatus::NotRequired
                    },
                    &statement_identifier,
                    &mut records,
                    &mut next_decision,
                ),
            }))
        } else {
            let common = DraftTaskCommon {
                owner: test_user_value(
                    request.owner,
                    &statement_identifier,
                    &mut records,
                    &mut next_decision,
                ),
                intent: test_intent_value(
                    intent,
                    &statement_identifier,
                    &mut records,
                    &mut next_decision,
                ),
                task_statement: test_user_value(
                    request.objective.clone(),
                    &statement_identifier,
                    &mut records,
                    &mut next_decision,
                ),
                execution_profile: test_execution_profile_value(
                    execution_profile,
                    &statement_identifier,
                    &mut records,
                    &mut next_decision,
                ),
                context_grants: test_optional_user_value(
                    context_grants,
                    &statement_identifier,
                    &mut records,
                    &mut next_decision,
                ),
                evidence_source_paths: test_optional_user_value(
                    evidence_source_paths,
                    &statement_identifier,
                    &mut records,
                    &mut next_decision,
                ),
            };
            match request.action {
                HarnessAction::CodeWrite | HarnessAction::DocumentWrite => {
                    let write_kind = if request.action == HarnessAction::CodeWrite {
                        WriteKind::Code
                    } else {
                        WriteKind::Document
                    };
                    DraftTaskRequest::Write(DraftWriteContract {
                        common,
                        write_kind: test_user_value(
                            write_kind,
                            &statement_identifier,
                            &mut records,
                            &mut next_decision,
                        ),
                        targets: test_user_value(
                            request.targets,
                            &statement_identifier,
                            &mut records,
                            &mut next_decision,
                        ),
                        delete_targets: test_optional_user_value(
                            request.delete_targets,
                            &statement_identifier,
                            &mut records,
                            &mut next_decision,
                        ),
                    })
                }
                HarnessAction::CodeReview | HarnessAction::DocumentReview => {
                    let review_kind = if request.action == HarnessAction::CodeReview {
                        ReviewKind::Code
                    } else {
                        ReviewKind::Document
                    };
                    DraftTaskRequest::Review(DraftReviewContract {
                        common,
                        review_kind: test_user_value(
                            review_kind,
                            &statement_identifier,
                            &mut records,
                            &mut next_decision,
                        ),
                        targets: test_user_value(
                            request.targets,
                            &statement_identifier,
                            &mut records,
                            &mut next_decision,
                        ),
                    })
                }
                HarnessAction::Investigation | HarnessAction::Design | HarnessAction::Ideation => {
                    let analysis_kind = match request.action {
                        HarnessAction::Investigation => AnalysisKind::Investigation,
                        HarnessAction::Design => AnalysisKind::Design,
                        HarnessAction::Ideation => AnalysisKind::Ideation,
                        _ => unreachable!(),
                    };
                    DraftTaskRequest::Analysis(DraftAnalysisContract {
                        common,
                        analysis_kind: test_user_value(
                            analysis_kind,
                            &statement_identifier,
                            &mut records,
                            &mut next_decision,
                        ),
                        targets: test_optional_user_value(
                            request.targets,
                            &statement_identifier,
                            &mut records,
                            &mut next_decision,
                        ),
                    })
                }
                HarnessAction::VaultRead | HarnessAction::VaultCuration => {
                    panic!("unsupported command test request")
                }
            }
        };
        let envelope = RequestEnvelope {
            version: HARNESS_SCHEMA_VERSION,
            source: RequestSource::UserLanguage {
                statements: vec![UserStatement {
                    identifier: statement_identifier,
                    text: request.objective,
                }],
            },
            draft,
            decision_trace: DecisionTrace { records },
        };
        let path = temp.path().join(format!("{name}-request-envelope.json"));
        write_test_json(&path, &envelope);
        path
    }

    fn test_user_value<T: serde::Serialize>(
        value: T,
        statement_identifier: &str,
        records: &mut Vec<DecisionRecord>,
        next_decision: &mut u64,
    ) -> DraftValue<T> {
        let decision_identifier = format!("decision-{:016x}", *next_decision);
        *next_decision += 1;
        records.push(DecisionRecord::UserStatement {
            identifier: decision_identifier.clone(),
            value_digest: test_serialized_digest(&value),
            statement_identifiers: vec![statement_identifier.to_owned()],
        });
        DraftValue::Resolved {
            value,
            decision_identifier,
        }
    }

    fn test_intent_value(
        value: HarnessIntent,
        statement_identifier: &str,
        records: &mut Vec<DecisionRecord>,
        next_decision: &mut u64,
    ) -> DraftValue<HarnessIntent> {
        if value == HarnessIntent::General {
            test_policy_value(
                value,
                PolicyDefaultRule::GeneralIntent,
                records,
                next_decision,
            )
        } else {
            test_user_value(value, statement_identifier, records, next_decision)
        }
    }

    fn test_execution_profile_value(
        value: HarnessExecutionProfile,
        statement_identifier: &str,
        records: &mut Vec<DecisionRecord>,
        next_decision: &mut u64,
    ) -> DraftValue<HarnessExecutionProfile> {
        if value == HarnessExecutionProfile::Standard {
            test_policy_value(
                value,
                PolicyDefaultRule::StandardExecutionProfile,
                records,
                next_decision,
            )
        } else {
            test_user_value(value, statement_identifier, records, next_decision)
        }
    }

    fn test_confirmation_value(
        value: UserConfirmationStatus,
        statement_identifier: &str,
        records: &mut Vec<DecisionRecord>,
        next_decision: &mut u64,
    ) -> DraftValue<UserConfirmationStatus> {
        if value == UserConfirmationStatus::NotRequired {
            test_policy_value(
                value,
                PolicyDefaultRule::NonJournalReadConfirmationNotRequired,
                records,
                next_decision,
            )
        } else {
            test_user_value(value, statement_identifier, records, next_decision)
        }
    }

    fn test_policy_value<T: serde::Serialize>(
        value: T,
        rule: PolicyDefaultRule,
        records: &mut Vec<DecisionRecord>,
        next_decision: &mut u64,
    ) -> DraftValue<T> {
        let decision_identifier = format!("decision-{:016x}", *next_decision);
        *next_decision += 1;
        records.push(DecisionRecord::PolicyDefault {
            identifier: decision_identifier.clone(),
            value_digest: test_serialized_digest(&value),
            policy_identifier: "control".to_owned(),
            policy_content_digest: test_configured_control_digest(),
            rule,
        });
        DraftValue::Resolved {
            value,
            decision_identifier,
        }
    }

    fn test_optional_user_value<T>(
        value: Vec<T>,
        statement_identifier: &str,
        records: &mut Vec<DecisionRecord>,
        next_decision: &mut u64,
    ) -> Option<DraftValue<Vec<T>>>
    where
        T: serde::Serialize,
    {
        if value.is_empty() {
            None
        } else {
            Some(test_user_value(
                value,
                statement_identifier,
                records,
                next_decision,
            ))
        }
    }

    #[cfg(unix)]
    fn codex_fixture(
        temp: &TempDirectory,
        documents: usize,
        repetitions: usize,
    ) -> (HarnessEngine, PreparedHarnessRun, Vec<u8>, PathBuf) {
        codex_fixture_for_action(temp, documents, repetitions, HarnessAction::DocumentReview)
    }

    #[cfg(unix)]
    fn codex_fixture_for_action(
        temp: &TempDirectory,
        documents: usize,
        repetitions: usize,
        action: HarnessAction,
    ) -> (HarnessEngine, PreparedHarnessRun, Vec<u8>, PathBuf) {
        codex_fixture_with_concurrency(temp, documents, repetitions, action, 1)
    }

    #[cfg(unix)]
    fn codex_fixture_with_concurrency(
        temp: &TempDirectory,
        documents: usize,
        repetitions: usize,
        action: HarnessAction,
        concurrency: usize,
    ) -> (HarnessEngine, PreparedHarnessRun, Vec<u8>, PathBuf) {
        let workspace = temp.path().join("workspace");
        fs::create_dir(&workspace).expect("synthetic workspace must be created");
        let extension = if matches!(action, HarnessAction::CodeReview | HarnessAction::CodeWrite) {
            "rs"
        } else {
            "md"
        };
        let targets = (0..documents)
            .map(|index| {
                let path = format!("document-{index}.{extension}");
                let content = format!(
                    "# 문서 {index}\n{}끝 🚀\n",
                    "한글🙂 e\u{301} \"quoted\" \\ path\t원문 줄\n".repeat(repetitions)
                );
                let content =
                    if matches!(action, HarnessAction::CodeReview | HarnessAction::CodeWrite) {
                        format!("pub const DOCUMENT: &str = {content:?};\n")
                    } else {
                        content
                    };
                fs::write(workspace.join(&path), content)
                    .expect("synthetic target must be written");
                path
            })
            .collect();
        let envelope_path = write_user_request_envelope(
            temp,
            "codex-review",
            HarnessRequest {
                action,
                owner: DataOwner::Profile,
                targets,
                objective: "evaluate exact frozen documents and proposed changes".to_owned(),
                curation_kind: None,
                explicit_user_confirmation_reported: false,
                curation_sources: Vec::new(),
                delete_targets: Vec::new(),
            },
            HarnessIntent::OutputAdapter {
                surface: CareerOutputSurface::General,
            },
            Vec::new(),
            Vec::new(),
            HarnessExecutionProfile::Strict,
        );
        let envelope = read_harness_json(&envelope_path, "synthetic review envelope")
            .expect("synthetic request must decode strictly");
        let repository = temp.path().join("source-placeholder");
        let router = HarnessRouter::with_execution_profile(2, HarnessExecutionProfile::Strict)
            .expect("strict router must be valid");
        let engine =
            test_engine(&repository, &workspace, router).expect("synthetic workspace must open");
        let resolved = engine
            .resolve_envelope(&envelope, None)
            .expect("review must resolve");
        let lifecycle = RoleLifecycleLimits {
            max_role_execution_millis: 10_000,
            max_role_grace_millis: 1_000,
            max_role_close_millis: 1_000,
            max_total_role_millis: 12_000,
        };
        let plan = HarnessPlan::from_resolved(resolved, &workspace, lifecycle.clone())
            .expect("synthetic documents must freeze");
        let mut capabilities = read_only_harness_capabilities(&plan, lifecycle);
        capabilities.max_concurrent_roles = concurrency;
        let raw_plan = serde_json::to_vec(&plan).expect("plan must serialize");
        let tool_plan = plan
            .requires_tool_execution()
            .then(|| codex_test_tool_plan(&plan, &workspace));
        let raw_tool_plan = tool_plan
            .as_ref()
            .map(|plan| serde_json::to_vec(plan).expect("tool plan must serialize"));
        let prepared = PreparedHarnessRun::prepare(
            &engine,
            &raw_plan,
            plan,
            capabilities,
            raw_tool_plan.as_deref(),
            tool_plan,
        )
        .expect("review must prepare with exactly its required tool plan");
        let raw = serde_json::to_vec(&prepared).expect("prepared run must serialize");
        (engine, prepared, raw, workspace)
    }

    #[cfg(unix)]
    fn codex_test_tool_plan(plan: &HarnessPlan, workspace: &Path) -> ToolExecutionPlan {
        fs::create_dir(workspace.join("checks")).expect("tool cwd must exist");
        let executable = "/usr/bin/true";
        let executable_bytes = fs::read(executable).expect("fixture executable must exist");
        let checks = plan
            .requirements
            .iter()
            .filter(|requirement| requirement.owner == VerificationOwner::Tool)
            .enumerate()
            .map(|(index, requirement)| ToolCheck {
                check_identifier: format!("synthetic-check-{index}"),
                verification_requirement: *requirement,
                executable: executable.to_owned(),
                executable_digest: test_byte_digest(&executable_bytes),
                argv: vec![executable.to_owned(), index.to_string()],
                cwd_workspace_relative: "checks".to_owned(),
                environment: std::collections::BTreeMap::new(),
                inherit_environment: false,
                stdin: ToolStdin {
                    bytes: Vec::new(),
                    bytes_digest: test_byte_digest(&[]),
                },
                timeout_millis: 1_000,
            })
            .collect();
        ToolExecutionPlan::build(
            plan.resolved_plan_digest.clone(),
            plan.workspace_root_identity.clone(),
            checks,
        )
        .expect("synthetic tool recipes must bind the resolved plan")
    }

    #[cfg(unix)]
    fn codex_test_tool_evidence(plan: &ToolExecutionPlan) -> ToolExecutionEvidence {
        ToolExecutionEvidence {
            version: HARNESS_SCHEMA_VERSION,
            tool_plan_digest: plan.tool_plan_digest.clone(),
            checks: plan
                .checks
                .iter()
                .cloned()
                .map(|check| ToolCheckEvidence {
                    check,
                    started_millis: 1,
                    finished_millis: 2,
                    exit_code: 0,
                    stdout_digest: test_byte_digest(&[]),
                    stderr_digest: test_byte_digest(&[]),
                    semantic_result: "passed".to_owned(),
                })
                .collect(),
        }
    }

    #[cfg(unix)]
    fn codex_test_tool_event(
        invocation: &ToolInvocationContract,
        exact: &ToolExecutionEvidence,
    ) -> HarnessExecutionEvent {
        let executions = exact
            .checks
            .iter()
            .map(|evidence| {
                let unit = evidence.check.verification_requirement.unit;
                let termination = ToolTermination::Exited {
                    code: evidence.exit_code,
                };
                ToolCommandEvidence {
                    unit,
                    command: evidence.check.argv.clone(),
                    evidence_digest: test_serialized_digest(&(
                        &invocation.invocation_digest,
                        unit,
                        &evidence.check.argv,
                        &termination,
                        &evidence.stdout_digest,
                        &evidence.stderr_digest,
                    )),
                    termination,
                    stdout_digest: evidence.stdout_digest.clone(),
                    stderr_digest: evidence.stderr_digest.clone(),
                }
            })
            .collect::<Vec<_>>();
        let results = invocation
            .requirements
            .iter()
            .map(|requirement| RequirementResult {
                unit: requirement.unit,
                passed: true,
                detail: "synthetic passed tool observation".to_owned(),
                evidence: executions
                    .iter()
                    .filter(|execution| execution.unit == requirement.unit)
                    .map(|execution| ResultEvidenceReference::ToolResult {
                        unit: execution.unit,
                        result_digest: execution.evidence_digest.clone(),
                    })
                    .collect(),
            })
            .collect::<Vec<_>>();
        HarnessExecutionEvent::ToolEvidence {
            evidence: ToolEvidenceSet {
                invocation_digest: invocation.invocation_digest.clone(),
                evidence_set_digest: test_serialized_digest(&(
                    &invocation.invocation_digest,
                    &results,
                    &executions,
                )),
                results,
                executions,
            },
        }
    }

    #[cfg(unix)]
    fn codex_completed(
        prepared: &PreparedHarnessRun,
        role: HarnessRole,
        failed: bool,
    ) -> RoleExecutionOutcome {
        let evidence: Vec<_> = prepared
            .plan
            .resolved_request
            .plan
            .targets
            .iter()
            .map(|target| {
                let TargetState::Existing { content_digest } = &target.state else {
                    panic!("review fixture targets must exist");
                };
                ResultEvidenceReference::Target {
                    workspace_relative_path: target.workspace_relative_path.clone(),
                    content_digest: content_digest.clone(),
                    locator: "entire document".to_owned(),
                }
            })
            .collect();
        codex_completed_with_evidence(prepared, role, failed, evidence)
    }

    #[cfg(unix)]
    fn codex_completed_with_evidence(
        prepared: &PreparedHarnessRun,
        role: HarnessRole,
        failed: bool,
        evidence: Vec<ResultEvidenceReference>,
    ) -> RoleExecutionOutcome {
        let metadata = prepared
            .role_run
            .role_metadata
            .iter()
            .find(|metadata| metadata.role == role)
            .expect("every invoked role must have prepared metadata");
        let (RoleTaskContract::Verifier {
            verification_requirements,
            ..
        }
        | RoleTaskContract::Reviewer {
            verification_requirements,
            ..
        }) = &metadata.task
        else {
            panic!("review fixture must contain only review roles");
        };
        let requirement_results = verification_requirements
            .iter()
            .enumerate()
            .map(|(index, requirement)| RequirementResult {
                unit: requirement.unit,
                passed: !(failed && index == 0),
                detail: "synthetic requirement observation".to_owned(),
                evidence: evidence.clone(),
            })
            .collect::<Vec<_>>();
        if failed {
            assert!(!requirement_results.is_empty());
        }
        let result = match role {
            HarnessRole::Verifier => CompletedRoleResult::Verifier {
                subject_evidence: evidence,
                requirement_results,
            },
            HarnessRole::Reviewer => CompletedRoleResult::Reviewer {
                summary: "No Findings".to_owned(),
                subject_evidence: evidence,
                requirement_results,
                blocking_findings: Vec::new(),
                improvements: Vec::new(),
                learning_candidates: Vec::new(),
            },
            _ => panic!("review fixture must contain only review roles"),
        };
        RoleExecutionOutcome::Completed { result }
    }

    #[cfg(unix)]
    fn codex_test_lifecycle(
        role: HarnessRole,
        context_id: String,
        outcome: &RoleExecutionOutcome,
    ) -> ReportedRoleLifecycle {
        ReportedRoleLifecycle {
            role,
            context_id,
            started_at_millis: 1,
            context_ready_at_millis: 2,
            first_output_at_millis: Some(3),
            interrupt_requested_at_millis: None,
            grace_deadline_at_millis: None,
            terminal_at_millis: 4,
            closed_at_millis: 5,
            terminal_state: codex_terminal_state(outcome),
        }
    }

    #[cfg(unix)]
    fn codex_test_script(temp: &TempDirectory, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let path = temp.path().join("fake-codex");
        let script = format!(
            r#"#!/bin/sh
printf '%s\n' "$$" >> "$0.calls"
printf '%s\n' "$PWD" > "$0.cwd.$$"
ls -ld "$PWD" >> "$0.cwd.$$"
printf '%s\n' "$@" > "$0.args.$$"
cp "${{10}}" "$0.schema.$$"
{body}
"#
        );
        fs::write(&path, script).expect("synthetic Codex must be written");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .expect("synthetic Codex must be executable");
        path
    }

    #[cfg(unix)]
    const CODEX_CAPTURE: &str = r#"cat > "$0.stdin.$$"
printf '{"type":"thread.started","thread_id":"native-%s"}\n' "$$"
n=0
while IFS= read -r entry; do n=$((n + 1)); done < "$0.calls"
cat "$0.events.$n"
"#;

    fn codex_test_usage() -> CodexUsage {
        CodexUsage {
            input: 100,
            cached_input: 20,
            cache_write_input: 0,
            output: 10,
            reasoning_output: 5,
        }
    }

    #[test]
    fn codex_observed_native_wire_round_trips_and_rejects_unknown_or_omitted_fields() {
        let wire = [
            r#"{"type":"thread.started","thread_id":"observed-thread"}"#,
            r#"{"type":"turn.started"}"#,
            r#"{"type":"item.started","item":{"id":"item_0","type":"command_execution","command":"pwd","aggregated_output":"","exit_code":null,"status":"in_progress"}}"#,
            r#"{"type":"item.completed","item":{"id":"item_0","type":"command_execution","command":"pwd","aggregated_output":"/tmp\n","exit_code":0,"status":"completed"}}"#,
            r#"{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"관측 원문 🙂"}}"#,
            r#"{"type":"turn.completed","usage":{"input_tokens":100,"cached_input_tokens":20,"cache_write_input_tokens":0,"output_tokens":10,"reasoning_output_tokens":5}}"#,
        ];
        let mut native = CodexNativeOutput::default();
        for (index, text) in wire.iter().enumerate() {
            let event: CodexEvent = decode_current_json(text.as_bytes(), "observed native wire")
                .expect("all observed native fields must decode strictly");
            let encoded = serde_json::to_vec(&event).expect("event must serialize");
            assert_eq!(
                decode_current_json::<CodexEvent>(&encoded, "native round trip")
                    .expect("current event must round trip"),
                event
            );
            native
                .observe(
                    text.as_bytes(),
                    u64::try_from(index + 1).expect("small index"),
                )
                .expect("observed native sequence must be accepted");
            // Every current field is required, including null exit_code and nested usage.
            for field in match &event {
                CodexEvent::ThreadStarted { .. } => vec!["thread_id"],
                CodexEvent::TurnStarted => Vec::new(),
                CodexEvent::ItemStarted { .. } | CodexEvent::ItemCompleted { .. } => {
                    vec!["item", "id", "type"]
                }
                CodexEvent::TurnCompleted { .. } => vec![
                    "usage",
                    "input_tokens",
                    "cached_input_tokens",
                    "cache_write_input_tokens",
                    "output_tokens",
                    "reasoning_output_tokens",
                ],
            } {
                let changed = text.replacen(&format!("\"{field}\":"), "\"unknown_field\":", 1);
                assert!(
                    decode_current_json::<CodexEvent>(changed.as_bytes(), "renamed native field")
                        .is_err()
                );
            }
            let extra = format!(
                "{},\"extra\":true}}",
                text.strip_suffix('}').expect("object wire")
            );
            assert!(
                decode_current_json::<CodexEvent>(extra.as_bytes(), "extra native field").is_err()
            );
        }
        assert_eq!(native.first_output, Some(5));
        assert_eq!(native.terminal, Some((6, true)));
        assert_eq!(native.usage, Some(codex_test_usage()));
        for invalid in [
            r#"{"type":"unknown.event"}"#,
            r#"{"type":"thread.started"}"#,
            r#"{"type":"turn.completed"}"#,
            r#"{"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":1}}"#,
            r#"{"type":"item.completed","item":{"id":"x","type":"agent_message"}}"#,
            r#"{"type":"item.started","item":{"id":"x","type":"command_execution","command":"pwd","aggregated_output":"","status":"in_progress"}}"#,
            r#"{"type":"item.completed","item":{"id":"x","type":"command_execution","command":"pwd","aggregated_output":"","exit_code":0,"status":"completed","extra":true}}"#,
            r#"{"type":"item.completed","item":{"id":"x","type":"unknown_item"}}"#,
        ] {
            assert!(
                decode_current_json::<CodexEvent>(invalid.as_bytes(), "invalid native wire")
                    .is_err()
            );
        }
    }

    #[cfg(unix)]
    fn codex_test_events(binary: &Path, number: usize, outcome: RoleExecutionOutcome) {
        let text =
            serde_json::to_string(&CodexResponse { outcome }).expect("outcome must serialize");
        let events = codex_test_jsonl(&[
            CodexEvent::TurnStarted,
            CodexEvent::ItemCompleted {
                item: CodexItem::AgentMessage {
                    id: "message".to_owned(),
                    text,
                },
            },
            CodexEvent::TurnCompleted {
                usage: codex_test_usage(),
            },
        ]);
        fs::write(format!("{}.events.{number}", binary.display()), events)
            .expect("native events must be written");
    }

    #[cfg(unix)]
    fn codex_test_jsonl(events: &[CodexEvent]) -> String {
        let mut output = String::new();
        for event in events {
            writeln!(
                output,
                "{}",
                serde_json::to_string(event).expect("native event must serialize")
            )
            .expect("writing native JSONL to a String must succeed");
        }
        output
    }

    #[cfg(unix)]
    fn codex_calls(binary: &Path) -> Vec<String> {
        fs::read_to_string(format!("{}.calls", binary.display()))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[cfg(unix)]
    fn codex_prepare_success_events(
        engine: &HarnessEngine,
        prepared: &PreparedHarnessRun,
        binary: &Path,
    ) {
        let mut step = engine
            .begin_execution(&prepared.plan.resolved_request, &prepared.role_run, &[])
            .expect("fixture execution must begin");
        let mut number = 0;
        while let Some(invocation) = step.ready_role_invocations.first() {
            number += 1;
            let outcome = codex_completed(
                prepared,
                invocation.role,
                invocation.role == HarnessRole::Reviewer,
            );
            codex_test_events(binary, number, outcome.clone());
            let lifecycle =
                codex_test_lifecycle(invocation.role, format!("fixture-{number}"), &outcome);
            let result = invocation
                .bind_result(lifecycle, outcome)
                .expect("fixture result must bind");
            step = engine
                .advance_execution(
                    &prepared.plan.resolved_request,
                    &prepared.role_run,
                    &[],
                    &step.record,
                    HarnessExecutionEvent::RoleResult {
                        result: Box::new(result),
                    },
                )
                .expect("fixture frontier must advance");
        }
        assert_eq!(
            number, 2,
            "strict review must exercise both Reviewer and Verifier"
        );
        assert!(step.ready_tool_invocation.is_none());
    }

    #[cfg(unix)]
    fn codex_assert_process_isolation(binary: &Path, pid: &str) -> String {
        let cwd = fs::read_to_string(format!("{}.cwd.{pid}", binary.display()))
            .expect("cwd must be captured");
        let mut lines = cwd.lines();
        let cwd_path = lines.next().expect("cwd path must be present");
        assert!(
            lines
                .next()
                .expect("cwd mode must be present")
                .starts_with("drwx------")
        );
        assert!(
            !Path::new(cwd_path).exists(),
            "temporary cwd must be removed after reaping"
        );
        let args = fs::read_to_string(format!("{}.args.{pid}", binary.display()))
            .expect("args must be captured");
        let args = args.lines().collect::<Vec<_>>();
        assert_eq!(
            &args[..9],
            [
                "exec",
                "--ephemeral",
                "--skip-git-repo-check",
                "--json",
                "--color",
                "never",
                "--sandbox",
                "read-only",
                "--output-schema"
            ]
        );
        assert_eq!(args.len(), 11);
        assert_eq!(args[10], "-");
        assert_eq!(Path::new(args[9]).parent(), Some(Path::new(cwd_path)));
        let schema = fs::read(format!("{}.schema.{pid}", binary.display()))
            .expect("schema must be captured");
        assert_eq!(schema, codex_output_schema().as_bytes());
        cwd_path.to_owned()
    }

    #[cfg(unix)]
    fn codex_assert_exact_transport(
        engine: &HarnessEngine,
        prepared: &PreparedHarnessRun,
        head: &HarnessExecutionRecord,
        binary: &Path,
    ) -> usize {
        let calls = codex_calls(binary);
        assert_eq!(
            calls.len(),
            head.role_execution.accepted_role_order.len(),
            "one process per accepted role and zero automatic retries at each document size"
        );
        let mut step = engine
            .begin_execution(&prepared.plan.resolved_request, &prepared.role_run, &[])
            .expect("Core replay must begin");
        let mut cwds = std::collections::BTreeSet::new();
        let mut transmitted = 0;
        for (role, pid) in head.role_execution.accepted_role_order.iter().zip(&calls) {
            let invocation = step
                .ready_role_invocations
                .iter()
                .find(|invocation| invocation.role == *role)
                .expect("accepted role must come from the latest frontier");
            let expected =
                serde_json::to_vec(&invocation.segments).expect("segments must serialize");
            let actual = fs::read(format!("{}.stdin.{pid}", binary.display()))
                .expect("stdin must be captured");
            assert_eq!(actual.len(), expected.len());
            assert!(
                actual == expected,
                "complete ordered segments must be byte exact"
            );
            transmitted += actual.len();
            assert!(
                cwds.insert(codex_assert_process_isolation(binary, pid)),
                "each role must use a fresh cwd"
            );
            let result = head
                .role_execution
                .role_results
                .iter()
                .find(|result| result.role == *role)
                .expect("accepted result must exist");
            assert_eq!(result.context_id, format!("native-{pid}"));
            assert_eq!(
                result.result_digest,
                test_serialized_digest(&(
                    result.role,
                    &result.invocation_digest,
                    &result.context_id,
                    &result.lifecycle,
                    &result.outcome,
                ))
            );
            step = engine
                .advance_execution(
                    &prepared.plan.resolved_request,
                    &prepared.role_run,
                    &[],
                    &step.record,
                    HarnessExecutionEvent::RoleResult {
                        result: Box::new(result.clone()),
                    },
                )
                .expect("native result must replay in Core");
        }
        transmitted
    }

    #[cfg(unix)]
    #[test]
    fn codex_begin_transports_exact_segments_at_two_document_sizes_and_preserves_core_rejection() {
        use context_core::harness::{EvaluatedArtifact, SubjectStatus};
        for (documents, repetitions) in [(2, 2048), (3, 4096)] {
            let temp = TempDirectory::new("codex-exact-transport");
            let (engine, prepared, raw, workspace) = codex_fixture(&temp, documents, repetitions);
            let binary = codex_test_script(&temp, CODEX_CAPTURE);
            codex_prepare_success_events(&engine, &prepared, &binary);
            let prepared_path = temp.path().join("prepared.json");
            fs::write(&prepared_path, &raw).expect("prepared bytes must be persisted exactly");
            let repository = temp.path().join("source-placeholder");
            run_begin_harness_command([
                "--context-view".to_owned(),
                repository.to_string_lossy().into_owned(),
                "--workspace-root".to_owned(),
                workspace.to_string_lossy().into_owned(),
                "--prepared-run".to_owned(),
                prepared_path.to_string_lossy().into_owned(),
                "--run-id".to_owned(),
                "native-review".to_owned(),
                "--codex-binary".to_owned(),
                binary.to_string_lossy().into_owned(),
                "--json".to_owned(),
            ])
            .expect("begin must execute and evaluate both native review roles");
            let head: HarnessExecutionRecord = read_harness_json(
                &workspace.join(".llm-context-vault-harness/runs/native-review/head.json"),
                "evaluated head",
            )
            .expect("durable head must decode strictly");
            assert_eq!(head.state, HarnessExecutionState::Evaluated);
            assert!(head.validation_receipt_digest.is_none());
            assert!(head.finalization_digest.is_none());
            let evaluation = head
                .evaluation
                .as_ref()
                .expect("Core evaluation must be present");
            assert_eq!(evaluation.subject_status, SubjectStatus::Rejected);
            assert!(evaluation.requirements.iter().any(|result| !result.passed));
            assert!(
                matches!(&evaluation.artifact, Some(EvaluatedArtifact::Review { summary }) if summary.starts_with("Rejected:"))
            );
            let transmitted = codex_assert_exact_transport(&engine, &prepared, &head, &binary);
            assert!(
                transmitted > documents * repetitions * 30,
                "transport must include long Unicode bodies"
            );
            head.validate(&prepared, &raw)
                .expect("evaluation must remain exactly Core-derived");
            for target in &prepared.plan.resolved_request.plan.targets {
                assert!(workspace.join(&target.workspace_relative_path).is_file());
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn codex_advance_retains_the_json_only_path_and_runs_only_the_latest_frontier() {
        let temp = TempDirectory::new("codex-advance");
        let (engine, prepared, raw, workspace) = codex_fixture(&temp, 2, 8);
        let prepared_path = temp.path().join("prepared.json");
        fs::write(&prepared_path, &raw).expect("prepared file must be written");
        let repository = temp.path().join("source-placeholder");
        run_begin_harness_command([
            "--context-view".to_owned(),
            repository.to_string_lossy().into_owned(),
            "--workspace-root".to_owned(),
            workspace.to_string_lossy().into_owned(),
            "--prepared-run".to_owned(),
            prepared_path.to_string_lossy().into_owned(),
            "--run-id".to_owned(),
            "advance-review".to_owned(),
            "--json".to_owned(),
        ])
        .expect("begin without Codex must keep the JSON-only path");
        let head_path = workspace.join(".llm-context-vault-harness/runs/advance-review/head.json");
        let head: HarnessExecutionRecord =
            read_harness_json(&head_path, "begun head").expect("head must decode");
        assert_eq!(head.state, HarnessExecutionState::Begun);
        assert!(head.role_execution.role_results.is_empty());
        let invocation = &head.ready_role_invocations[0];
        let outcome = codex_completed(&prepared, invocation.role, false);
        let lifecycle =
            codex_test_lifecycle(invocation.role, "external-first-role".to_owned(), &outcome);
        let event = HarnessExecutionEvent::RoleResult {
            result: Box::new(
                invocation
                    .bind_result(lifecycle, outcome)
                    .expect("result must bind"),
            ),
        };
        let preview = head
            .advance(&engine, &raw, &prepared, event.clone())
            .expect("event must yield a new frontier");
        let binary = codex_test_script(&temp, CODEX_CAPTURE);
        assert_eq!(preview.ready_role_invocations.len(), 1);
        codex_test_events(
            &binary,
            1,
            codex_completed(&prepared, preview.ready_role_invocations[0].role, false),
        );
        let event_path = temp.path().join("event.json");
        write_test_json(&event_path, &event);
        run_advance_harness_command([
            "--context-view".to_owned(),
            repository.to_string_lossy().into_owned(),
            "--workspace-root".to_owned(),
            workspace.to_string_lossy().into_owned(),
            "--run-id".to_owned(),
            "advance-review".to_owned(),
            "--event".to_owned(),
            event_path.to_string_lossy().into_owned(),
            "--codex-binary".to_owned(),
            binary.to_string_lossy().into_owned(),
            "--json".to_owned(),
        ])
        .expect("advance must execute only its returned frontier");
        assert_eq!(codex_calls(&binary).len(), 1);
        let head: HarnessExecutionRecord =
            read_harness_json(&head_path, "evaluated head").expect("head must decode");
        assert_eq!(head.state, HarnessExecutionState::Evaluated);
        assert_eq!(head.role_execution.role_results.len(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn codex_tool_plan_waits_for_evidence_then_returns_head_for_manual_evaluation() {
        let temp = TempDirectory::new("codex-tool-evidence");
        let (engine, prepared, raw, workspace) =
            codex_fixture_for_action(&temp, 2, 8, HarnessAction::CodeReview);
        let binary = codex_test_script(&temp, CODEX_CAPTURE);
        let head =
            HarnessExecutionRecord::begin_durable(&engine, &raw, &prepared, "native-tool-review")
                .expect("tool-backed review must begin");
        let plan = prepared
            .accepted_tool_plan
            .as_ref()
            .expect("code review must have an accepted tool plan");
        let invocation = head
            .ready_tool_invocation
            .as_ref()
            .expect("code review must initially issue its tool frontier");
        let blocked = execute_codex_frontier(
            &engine,
            &prepared,
            head.clone(),
            &binary,
            None,
            &RoleObservations::default(),
        )
        .expect_err("native roles must not run while a tool invocation is ready");
        assert!(blocked.to_string().contains("unsupported runtime"));
        assert!(codex_calls(&binary).is_empty());
        let exact = codex_test_tool_evidence(plan);
        let head = HarnessExecutionRecord::advance_durable(
            &engine,
            &head.run_identifier,
            codex_test_tool_event(invocation, &exact),
        )
        .expect("Core must accept evidence bound to its issued tool invocation");
        assert!(head.ready_tool_invocation.is_none());
        assert_eq!(head.ready_role_invocations.len(), 2);
        for (index, invocation) in head.ready_role_invocations.iter().enumerate() {
            codex_test_events(
                &binary,
                index + 1,
                codex_completed(&prepared, invocation.role, false),
            );
        }
        let next = execute_codex_frontier(
            &engine,
            &prepared,
            head,
            &binary,
            None,
            &RoleObservations::default(),
        )
        .expect("native review must return the current head without autoevaluation");
        assert_eq!(codex_calls(&binary).len(), 2);
        assert_eq!(next.state, HarnessExecutionState::Executing);
        assert_eq!(next.role_execution.role_results.len(), 2);
        assert!(next.ready_role_invocations.is_empty());
        assert!(next.ready_tool_invocation.is_none());
        assert!(next.evaluation.is_none());
        assert!(next.exact_tool_evidence.is_none());
        assert!(next.run_evaluation_receipt_digest.is_none());
        let stored: HarnessExecutionRecord = read_harness_json(
            &workspace.join(".llm-context-vault-harness/runs/native-tool-review/head.json"),
            "current unevaluated tool head",
        )
        .expect("current head must decode strictly");
        assert_eq!(
            next, stored,
            "native helper must return the latest durable head"
        );
        next.validate(&prepared, &raw)
            .expect("returned head must remain bound to the accepted preparation");
        codex_assert_manual_tool_evaluation(&engine, &next.run_identifier, &exact);
    }

    #[cfg(unix)]
    fn codex_assert_manual_tool_evaluation(
        engine: &HarnessEngine,
        run_identifier: &str,
        exact: &ToolExecutionEvidence,
    ) {
        use context_core::harness::SubjectStatus;

        assert!(
            HarnessExecutionRecord::evaluate_durable(engine, run_identifier, None).is_err(),
            "recorded ToolEvidenceSet does not replace exact tool execution evidence"
        );
        let mut mismatched = exact.clone();
        mismatched.checks[0].stdout_digest = test_byte_digest(b"other output");
        assert!(
            HarnessExecutionRecord::evaluate_durable(engine, run_identifier, Some(&mismatched))
                .is_err(),
            "evaluation must cross-check exact evidence against the recorded tool observations"
        );
        let evaluated =
            HarnessExecutionRecord::evaluate_durable(engine, run_identifier, Some(exact))
                .expect("the same bound tool evidence must permit Core evaluation");
        assert_eq!(evaluated.state, HarnessExecutionState::Evaluated);
        assert_eq!(evaluated.exact_tool_evidence.as_ref(), Some(exact));
        let evaluation = evaluated
            .evaluation
            .expect("Core evaluation must be present");
        assert_eq!(evaluation.subject_status, SubjectStatus::Accepted);
        assert!(evaluation.requirements.iter().all(|result| result.passed));
        assert!(evaluated.validation_receipt_digest.is_none());
        assert!(evaluated.finalization_digest.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn codex_terminal_results_stop_ready_siblings_without_retries() {
        for outcome in [
            RoleExecutionOutcome::Failed {
                message: "failed fixture".to_owned(),
            },
            RoleExecutionOutcome::Cancelled,
            RoleExecutionOutcome::TimedOut,
            RoleExecutionOutcome::Unsupported {
                reason: "unsupported fixture".to_owned(),
            },
        ] {
            let temp = TempDirectory::new("codex-halt");
            let (engine, prepared, raw, _) = codex_fixture(&temp, 2, 8);
            let binary = codex_test_script(&temp, CODEX_CAPTURE);
            codex_test_events(&binary, 1, outcome.clone());
            let head =
                HarnessExecutionRecord::begin_durable(&engine, &raw, &prepared, "halt-review")
                    .expect("run must begin");
            assert_eq!(
                head.ready_role_invocations.len(),
                2,
                "test must start with ready siblings"
            );
            let evaluated = execute_codex_frontier(
                &engine,
                &prepared,
                head,
                &binary,
                None,
                &RoleObservations::default(),
            )
            .expect("terminal result must be evaluated by Core");
            assert_eq!(codex_calls(&binary).len(), 1);
            assert_eq!(evaluated.role_execution.role_results.len(), 1);
            assert_eq!(evaluated.role_execution.role_results[0].outcome, outcome);
            assert!(evaluated.ready_role_invocations.is_empty());
        }
    }

    #[cfg(unix)]
    const CODEX_CONCURRENT_CAPTURE: &str = r#"cat > "$0.stdin.$$"
role=$(python3 -c 'import json,sys; print(json.loads(json.load(open(sys.argv[1]))[0]["content"])["role"])' "$0.stdin.$$")
printf '%s\n' "$$" > "$0.pid.$role"
printf '{"type":"thread.started","thread_id":"native-%s"}\n' "$$"
# Both issued siblings must start before either result; a sequential adapter fails this barrier.
i=0
while [ ! -f "$0.pid.verifier" ] || [ ! -f "$0.pid.reviewer" ]; do
  i=$((i + 1)); [ "$i" -lt 300 ] || exit 9
  sleep 0.01
done
sleep "$(cat "$0.delay.$role")" &
printf '%s\n' "$!" > "$0.descendant.$role"
wait
cat "$0.events.$role"
"#;

    #[cfg(unix)]
    fn codex_concurrent_fixture(
        temp: &TempDirectory,
        prepared: &PreparedHarnessRun,
        reviewer: RoleExecutionOutcome,
        verifier: RoleExecutionOutcome,
        reviewer_delay: &str,
        verifier_delay: &str,
    ) -> PathBuf {
        let binary = codex_test_script(temp, CODEX_CONCURRENT_CAPTURE);
        for (number, role, outcome, delay) in [
            (1, "verifier", verifier, verifier_delay),
            (2, "reviewer", reviewer, reviewer_delay),
        ] {
            codex_test_events(&binary, number, outcome);
            fs::rename(
                format!("{}.events.{number}", binary.display()),
                format!("{}.events.{role}", binary.display()),
            )
            .expect("role-specific response must be installed");
            fs::write(format!("{}.delay.{role}", binary.display()), delay)
                .expect("role delay must be installed");
        }
        assert_eq!(
            prepared.role_run.runtime_capabilities.max_concurrent_roles,
            2
        );
        binary
    }

    #[cfg(unix)]
    fn codex_assert_concurrent_processes_closed(binary: &Path) {
        for role in ["verifier", "reviewer"] {
            let pid = fs::read_to_string(format!("{}.pid.{role}", binary.display()))
                .expect("both siblings must launch");
            let group: i32 = pid.trim().parse().expect("owned PID");
            // SAFETY: signal zero only observes the positive owned process group.
            assert_eq!(unsafe { libc::kill(-group, 0) }, -1);
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ESRCH)
            );
            codex_assert_process_isolation(binary, pid.trim());
        }
        assert_eq!(
            codex_calls(binary).len(),
            2,
            "no role retry or extra process"
        );
    }

    #[cfg(unix)]
    #[test]
    fn codex_concurrent_siblings_keep_exact_inputs_and_accept_completion_order() {
        let temp = TempDirectory::new("codex-concurrent-exact");
        let (engine, prepared, raw, _) =
            codex_fixture_with_concurrency(&temp, 2, 8, HarnessAction::DocumentReview, 2);
        let binary = codex_concurrent_fixture(
            &temp,
            &prepared,
            codex_completed(&prepared, HarnessRole::Reviewer, false),
            codex_completed(&prepared, HarnessRole::Verifier, false),
            "0.04",
            "0.3",
        );
        let head =
            HarnessExecutionRecord::begin_durable(&engine, &raw, &prepared, "parallel-exact")
                .expect("run must begin");
        let issued = head.ready_role_invocations.clone();
        let evaluated = execute_codex_frontier(
            &engine,
            &prepared,
            head,
            &binary,
            None,
            &RoleObservations::default(),
        )
        .expect("both independent results must evaluate");
        assert_eq!(
            evaluated.role_execution.accepted_role_order,
            [HarnessRole::Reviewer, HarnessRole::Verifier]
        );
        assert_eq!(evaluated.role_execution.role_results.len(), 2);
        for invocation in issued {
            let role = if invocation.role == HarnessRole::Reviewer {
                "reviewer"
            } else {
                "verifier"
            };
            let pid = fs::read_to_string(format!("{}.pid.{role}", binary.display())).expect("PID");
            let actual =
                fs::read(format!("{}.stdin.{}", binary.display(), pid.trim())).expect("input");
            assert_eq!(
                actual,
                serde_json::to_vec(&invocation.segments).expect("issued segments")
            );
        }
        evaluated
            .validate(&prepared, &raw)
            .expect("Core head must remain valid");
        codex_assert_concurrent_processes_closed(&binary);
    }

    #[cfg(unix)]
    #[test]
    fn codex_concurrent_terminal_result_cancels_and_reaps_running_sibling() {
        let temp = TempDirectory::new("codex-concurrent-halt");
        let (engine, prepared, raw, workspace) =
            codex_fixture_with_concurrency(&temp, 2, 8, HarnessAction::DocumentReview, 2);
        let failure = RoleExecutionOutcome::Failed {
            message: "owned terminal fixture".to_owned(),
        };
        let binary = codex_concurrent_fixture(
            &temp,
            &prepared,
            failure.clone(),
            codex_completed(&prepared, HarnessRole::Verifier, false),
            "0.04",
            "30",
        );
        let head = HarnessExecutionRecord::begin_durable(&engine, &raw, &prepared, "parallel-halt")
            .expect("run must begin");
        let started = std::time::Instant::now();
        let evaluated = execute_codex_frontier(
            &engine,
            &prepared,
            head,
            &binary,
            Some(&test_source(&workspace)),
            &RoleObservations::default(),
        )
        .expect("first terminal must evaluate after closing sibling");
        assert!(started.elapsed() < std::time::Duration::from_secs(4));
        assert_eq!(evaluated.role_execution.role_results.len(), 1);
        assert_eq!(evaluated.role_execution.role_results[0].outcome, failure);
        assert!(evaluated.ready_role_invocations.is_empty());
        codex_assert_concurrent_processes_closed(&binary);
    }

    #[cfg(unix)]
    #[test]
    fn codex_concurrent_invalid_submission_stops_sibling_and_preserves_head() {
        let temp = TempDirectory::new("codex-concurrent-invalid");
        let (engine, prepared, raw, workspace) =
            codex_fixture_with_concurrency(&temp, 2, 8, HarnessAction::DocumentReview, 2);
        let binary = codex_concurrent_fixture(
            &temp,
            &prepared,
            codex_completed(&prepared, HarnessRole::Verifier, false),
            codex_completed(&prepared, HarnessRole::Verifier, false),
            "0.04",
            "30",
        );
        let head =
            HarnessExecutionRecord::begin_durable(&engine, &raw, &prepared, "parallel-invalid")
                .expect("run must begin");
        assert!(
            execute_codex_frontier(
                &engine,
                &prepared,
                head.clone(),
                &binary,
                None,
                &RoleObservations::default()
            )
            .is_err()
        );
        let saved: HarnessExecutionRecord = read_harness_json(
            &workspace.join(".llm-context-vault-harness/runs/parallel-invalid/head.json"),
            "head",
        )
        .expect("head must remain readable");
        assert_eq!(saved, head);
        codex_assert_concurrent_processes_closed(&binary);
    }

    #[cfg(unix)]
    #[test]
    fn codex_concurrent_later_terminal_preserves_the_already_accepted_sibling() {
        let temp = TempDirectory::new("codex-concurrent-late-halt");
        let (engine, prepared, raw, _) =
            codex_fixture_with_concurrency(&temp, 2, 8, HarnessAction::DocumentReview, 2);
        let binary = codex_concurrent_fixture(
            &temp,
            &prepared,
            codex_completed(&prepared, HarnessRole::Reviewer, false),
            RoleExecutionOutcome::Failed {
                message: "late terminal".to_owned(),
            },
            "0.04",
            "0.3",
        );
        let head =
            HarnessExecutionRecord::begin_durable(&engine, &raw, &prepared, "parallel-late-halt")
                .expect("run must begin");
        let evaluated = execute_codex_frontier(
            &engine,
            &prepared,
            head,
            &binary,
            None,
            &RoleObservations::default(),
        )
        .expect("prior successful sibling must persist");
        assert_eq!(
            evaluated.role_execution.accepted_role_order,
            [HarnessRole::Reviewer, HarnessRole::Verifier]
        );
        for result in &evaluated.role_execution.role_results {
            match result.role {
                HarnessRole::Reviewer => assert!(matches!(
                    result.outcome,
                    RoleExecutionOutcome::Completed { .. }
                )),
                HarnessRole::Verifier => assert!(matches!(
                    result.outcome,
                    RoleExecutionOutcome::Failed { .. }
                )),
                _ => panic!("only the issued siblings may be accepted"),
            }
        }
        codex_assert_concurrent_processes_closed(&binary);
    }

    #[cfg(unix)]
    #[test]
    fn codex_binding_defers_role_and_lifecycle_acceptance_to_core() {
        let temp = TempDirectory::new("codex-role-mismatch");
        let (engine, prepared, raw, workspace) = codex_fixture(&temp, 2, 8);
        let binary = codex_test_script(&temp, CODEX_CAPTURE);
        let head = HarnessExecutionRecord::begin_durable(&engine, &raw, &prepared, "mismatch")
            .expect("run must begin");
        let invocation = &head.ready_role_invocations[0];
        let wrong_role = if invocation.role == HarnessRole::Reviewer {
            HarnessRole::Verifier
        } else {
            HarnessRole::Reviewer
        };
        let outcome = codex_completed(&prepared, wrong_role, false);
        let bound = invocation
            .bind_result(
                codex_test_lifecycle(wrong_role, "mismatched-context".to_owned(), &outcome),
                outcome.clone(),
            )
            .expect("binding alone must not duplicate acceptance validation");
        assert!(
            HarnessExecutionRecord::advance_durable(
                &engine,
                "mismatch",
                HarnessExecutionEvent::RoleResult {
                    result: Box::new(bound)
                }
            )
            .is_err()
        );
        codex_test_events(&binary, 1, outcome);
        assert!(
            execute_codex_frontier(
                &engine,
                &prepared,
                head.clone(),
                &binary,
                None,
                &RoleObservations::default()
            )
            .is_err()
        );
        assert_eq!(
            codex_calls(&binary).len(),
            1,
            "role mismatch must not retry or run its sibling"
        );
        let persisted: HarnessExecutionRecord = read_harness_json(
            &workspace.join(".llm-context-vault-harness/runs/mismatch/head.json"),
            "unchanged head",
        )
        .expect("head must remain valid");
        assert_eq!(persisted, head);
    }

    #[cfg(unix)]
    #[test]
    fn codex_rejects_unsupported_tool_frontier_before_any_process() {
        let temp = TempDirectory::new("codex-unsupported-frontier");
        let (engine, prepared, raw, _) = codex_fixture(&temp, 2, 8);
        let binary = codex_test_script(&temp, CODEX_CAPTURE);
        let head = HarnessExecutionRecord::begin_durable(&engine, &raw, &prepared, "unsupported")
            .expect("run must begin");
        let mut tool_frontier = head;
        let invocation = &tool_frontier.ready_role_invocations[0];
        tool_frontier.ready_tool_invocation = Some(context_core::harness::ToolInvocationContract {
            version: invocation.version,
            resolved_plan_digest: invocation.resolved_plan_digest.clone(),
            prepared_role_run_digest: invocation.prepared_role_run_digest.clone(),
            revision_contract_digest: invocation.revision_contract_digest.clone(),
            requirements: Vec::new(),
            subject: invocation.subject.clone(),
            invocation_digest: invocation.invocation_digest.clone(),
        });
        assert!(
            execute_codex_frontier(
                &engine,
                &prepared,
                tool_frontier,
                &binary,
                None,
                &RoleObservations::default()
            )
            .expect_err("tool frontier must be unsupported")
            .to_string()
            .contains("unsupported runtime")
        );
        assert!(codex_calls(&binary).is_empty());
        assert!(validate_codex_binary(Path::new("relative-codex")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn codex_native_protocol_and_strict_outcome_failures_are_terminal() {
        // Allow fixture startup latency; protocol validation is the subject here.
        let limits = RoleLifecycleLimits {
            max_role_execution_millis: 10_000,
            max_role_grace_millis: 200,
            max_role_close_millis: 500,
            max_total_role_millis: 10_700,
        };
        for (body, missing_context) in [
            (
                "cat > /dev/null\nprintf '%s\\n' '{\"type\":\"thread.started\",\"thread_id\":\"native-fixture\"}' '{\"type\":\"turn.started\"}' '{\"type\":\"turn.completed\"}'",
                false,
            ),
            (
                "cat > /dev/null\nprintf '%s\\n' '{\"type\":\"thread.started\",\"thread_id\":\"native-fixture\"}' 'bad-json'",
                false,
            ),
            (
                "cat > /dev/null\nprintf '%s\\n' '{\"type\":\"thread.started\",\"thread_id\":\"native-fixture\"}' '{\"type\":\"turn.started\"}' '{\"type\":\"turn.failed\",\"error\":{\"message\":\"fixture\"}}'",
                false,
            ),
            ("cat > /dev/null\nexit 1", true),
            (
                "cat > /dev/null\nprintf '%s\\n' '{\"type\":\"turn.started\"}'",
                true,
            ),
        ] {
            let temp = TempDirectory::new("codex-native-failure");
            let binary = codex_test_script(&temp, body);
            let result = run_codex_process(
                &binary,
                b"[]",
                HarnessRole::Reviewer,
                &limits,
                &std::sync::atomic::AtomicBool::new(false),
            );
            if missing_context {
                let error = result
                    .expect_err("missing native context cannot be invented")
                    .to_string();
                assert!(error.contains("thread.started"), "{error}; fixture: {body}");
            } else {
                let (lifecycle, outcome) = result.unwrap_or_else(|error| {
                    panic!(
                        "observed failure must bind a native lifecycle: {error}; fixture: {body}"
                    )
                });
                assert!(matches!(outcome, RoleExecutionOutcome::Failed { .. }));
                assert_eq!(lifecycle.context_id, "native-fixture");
            }
            assert_eq!(codex_calls(&binary).len(), 1);
        }
        for text in [
            "No Findings",
            "{}",
            "{broken",
            "{\"outcome\":{\"status\":\"cancelled\",\"extra\":true}}",
            "{\"outcome\":{\"status\":\"cancelled\"},\"extra\":true}",
            "{\"outcome\":{\"status\":\"failed\"}}",
            "{\"outcome\":{\"status\":\"cancelled\",\"status\":\"timed-out\"}}",
        ] {
            let native = CodexNativeOutput {
                message: Some(text.to_owned()),
                terminal: Some((4, true)),
                ..CodexNativeOutput::default()
            };
            assert!(
                native.outcome(true).is_err(),
                "invalid Harness response must not become success"
            );
        }
        let mut duplicate = CodexNativeOutput::default();
        let thread = br#"{"type":"thread.started","thread_id":"observed"}"#;
        duplicate
            .observe(thread, 1)
            .expect("known native event must decode");
        assert!(duplicate.observe(thread, 2).is_err());
    }

    #[cfg(unix)]
    fn codex_stderr_fixture(binary: &Path) -> String {
        let stderr = format!(
            "DISCARDED-STDERR-PREFIX\n{}\nRETAINED-STDERR-TAIL 오류🙂\n",
            "0123456789abcdef".repeat(1024)
        );
        fs::write(format!("{}.stderr", binary.display()), &stderr)
            .expect("synthetic stderr must be written");
        stderr
    }

    #[cfg(unix)]
    fn codex_assert_bounded_stderr_diagnostic(message: &str, stderr: &str) {
        use std::os::unix::process::ExitStatusExt as _;

        let exit = process::ExitStatus::from_raw(23 << 8);
        assert!(
            message.contains(&exit.to_string()),
            "actual exit status must survive"
        );
        assert!(message.contains(stderr[stderr.len() - 4096..].trim()));
        assert!(!message.contains("DISCARDED-STDERR-PREFIX"));
        assert!(message.contains("RETAINED-STDERR-TAIL 오류🙂"));
        assert!(
            message.len() <= 4096 + 512,
            "the retained stderr and diagnostic envelope must have a finite length"
        );
    }

    #[cfg(unix)]
    #[test]
    fn codex_stderr_only_startup_failure_preserves_bounded_tail_and_exit_status() {
        let temp = TempDirectory::new("codex-stderr-startup");
        let binary = codex_test_script(&temp, "cat > /dev/null\ncat \"$0.stderr\" >&2\nexit 23\n");
        let stderr = codex_stderr_fixture(&binary);
        let limits = RoleLifecycleLimits {
            max_role_execution_millis: 3_000,
            max_role_grace_millis: 300,
            max_role_close_millis: 1_000,
            max_total_role_millis: 4_300,
        };
        let error = run_codex_process(
            &binary,
            b"[]",
            HarnessRole::Reviewer,
            &limits,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .expect_err("a stderr-only startup failure cannot invent a native context");
        let message = error.to_string();
        assert!(message.contains("thread.started"));
        codex_assert_bounded_stderr_diagnostic(&message, &stderr);
        assert_eq!(codex_calls(&binary).len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn codex_stderr_diagnoses_nonzero_exit_without_changing_valid_role_outcomes() {
        let fixture = TempDirectory::new("codex-stderr-outcomes");
        let (_, prepared, _, _) = codex_fixture(&fixture, 2, 8);
        let completed = codex_completed(&prepared, HarnessRole::Reviewer, false);
        for (exit_code, expected) in [
            (23, completed.clone()),
            (0, completed),
            (
                0,
                RoleExecutionOutcome::Failed {
                    message: "reported role failure".to_owned(),
                },
            ),
        ] {
            let temp = TempDirectory::new("codex-stderr-context");
            let body = format!("{CODEX_CAPTURE}\ncat \"$0.stderr\" >&2\nexit {exit_code}\n");
            let binary = codex_test_script(&temp, &body);
            let stderr = codex_stderr_fixture(&binary);
            codex_test_events(&binary, 1, expected.clone());
            let (lifecycle, outcome) = run_codex_process(
                &binary,
                b"[]",
                HarnessRole::Reviewer,
                &prepared.runtime_capabilities.lifecycle,
                &std::sync::atomic::AtomicBool::new(false),
            )
            .expect("observed native context must bind the failure or valid result");
            let calls = codex_calls(&binary);
            assert_eq!(calls.len(), 1);
            assert_eq!(lifecycle.context_id, format!("native-{}", calls[0]));
            if exit_code == 0 {
                assert_eq!(
                    outcome, expected,
                    "stderr must not alter a decoded role outcome"
                );
            } else {
                let RoleExecutionOutcome::Failed { message } = outcome else {
                    panic!("nonzero native exit must remain a transport failure");
                };
                assert_eq!(lifecycle.terminal_state, RoleTerminalState::Failed);
                codex_assert_bounded_stderr_diagnostic(&message, &stderr);
            }
        }
    }

    #[cfg(unix)]
    fn codex_write_scenario_outcome(
        prepared: &PreparedHarnessRun,
        invocation: &RoleInvocationContract,
        scenario: &str,
    ) -> RoleExecutionOutcome {
        use context_core::harness::{
            EvaluationSubject, FileChange, MissingContextCandidate, MissingContextRequest,
            WriterArtifact,
        };

        if invocation.role == HarnessRole::Writer {
            if scenario == "writer-failed" {
                return RoleExecutionOutcome::Failed {
                    message: "synthetic Writer failure before producing a candidate".to_owned(),
                };
            }
            let changes = prepared
                .plan
                .resolved_request
                .plan
                .targets
                .iter()
                .map(|target| {
                    let TargetState::Existing { content_digest } = &target.state else {
                        panic!("write fixture updates only existing documents");
                    };
                    FileChange::Update {
                        path: target.workspace_relative_path.clone(),
                        expected_content_digest: content_digest.clone(),
                        content: "# Synthetic proposed document\n".to_owned(),
                    }
                })
                .collect();
            return RoleExecutionOutcome::Completed {
                result: CompletedRoleResult::Writer {
                    artifact: WriterArtifact::Changes { changes },
                },
            };
        }
        let EvaluationSubject::ProducedArtifact { artifact_digest } = &invocation.subject else {
            panic!("write review must bind the Core-issued produced artifact");
        };
        let outcome = codex_completed_with_evidence(
            prepared,
            invocation.role,
            scenario == "completed-rejected" && invocation.role == HarnessRole::Reviewer,
            vec![ResultEvidenceReference::ProducedArtifact {
                artifact_digest: artifact_digest.clone(),
                locator: "entire produced artifact".to_owned(),
            }],
        );
        if scenario == "reviewer-missing-context" && invocation.role == HarnessRole::Reviewer {
            let RoleExecutionOutcome::Completed {
                result:
                    CompletedRoleResult::Reviewer {
                        subject_evidence,
                        requirement_results,
                        ..
                    },
            } = outcome
            else {
                panic!("Reviewer fixture must expose its exact subject and requirements");
            };
            return RoleExecutionOutcome::MissingContext {
                request: MissingContextRequest {
                    reason: "the proposed document requires an additional source".to_owned(),
                    blocked_verification_units: vec![requirement_results[0].unit],
                    subject_evidence,
                    candidate: MissingContextCandidate::AdditionalWorkspaceTarget {
                        workspace_relative_path: "additional.md".to_owned(),
                    },
                },
            };
        }
        outcome
    }

    #[cfg(unix)]
    fn codex_finish_write_scenario(
        engine: &HarnessEngine,
        prepared: &PreparedHarnessRun,
        mut head: HarnessExecutionRecord,
        scenario: &str,
    ) -> HarnessExecutionRecord {
        while let Some(invocation) = head.ready_role_invocations.first() {
            let outcome = codex_write_scenario_outcome(prepared, invocation, scenario);
            let lifecycle = codex_test_lifecycle(
                invocation.role,
                format!("{scenario}-{:?}", invocation.role),
                &outcome,
            );
            let result = invocation
                .bind_result(lifecycle, outcome)
                .expect("synthetic write result must bind the issued invocation");
            head = HarnessExecutionRecord::advance_durable(
                engine,
                &head.run_identifier,
                HarnessExecutionEvent::RoleResult {
                    result: Box::new(result),
                },
            )
            .expect("Core must accept the exact completed or halted write result");
        }
        assert!(head.ready_tool_invocation.is_none());
        head
    }

    #[cfg(unix)]
    #[test]
    fn codex_native_gate_cycles_refuse_suspended_reads_and_clear_verified_bodies() {
        let temp = TempDirectory::new("gate-cycles");
        let (_, _, _, workspace) = codex_fixture(&temp, 1, 1);
        // Gate idempotence needs a workload-local counter. Other parallel fixtures
        // intentionally share test_store(), and must not enter this observation.
        let store = test_runtime().block_on(async {
            Store::for_native(&std::env::var("TEST_DATABASE_URL").unwrap()).unwrap()
        });
        let view = workspace.join(".gate-test-view");
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(&view).unwrap();
        let source = test_runtime()
            .block_on(store.with_native_context(view, Ok))
            .unwrap();
        let path = Path::new("vault/profile/rules/control.md");
        source.suspend_reads().unwrap();
        source.resume_reads().unwrap();
        for _ in 0..3 {
            let before = source.body_queries();
            source.open_file(path, 65536).unwrap();
            source.open_file(path, 65536).unwrap();
            assert_eq!(source.body_queries(), before + 1);
            source.suspend_reads().unwrap();
            let calls = store.calls();
            source.suspend_reads().unwrap();
            assert_eq!(store.calls(), calls, "suspend is idempotent");
            assert!(source.metadata(Path::new("")).is_err());
            assert!(source.source_versions(&[]).is_err());
            assert!(source.children(Path::new(""), 1).is_err());
            assert!(source.store_identity().is_err());
            assert!(source.open_file(path, 65536).is_err());
            source.resume_reads().unwrap();
            let calls = store.calls();
            source.resume_reads().unwrap();
            assert_eq!(
                store.calls(),
                calls,
                "resume cannot accumulate shared locks"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn codex_producer_flows_through_independent_review_without_applying() {
        let temp = TempDirectory::new("codex-producer-flow");
        let (engine, prepared, raw, workspace) =
            codex_fixture_for_action(&temp, 2, 8, HarnessAction::DocumentWrite);
        let original = fs::read(workspace.join("document-0.md")).unwrap();
        let head = HarnessExecutionRecord::begin_durable(&engine, &raw, &prepared, "producer-flow")
            .unwrap();
        let binary = codex_test_script(&temp, CODEX_CAPTURE);
        let mut preview = head.clone();
        let mut number = 1;
        while let Some(invocation) = preview.ready_role_invocations.first() {
            let outcome = codex_write_scenario_outcome(&prepared, invocation, "accepted");
            codex_test_events(&binary, number, outcome.clone());
            number += 1;
            let lifecycle =
                codex_test_lifecycle(invocation.role, format!("preview-{number}"), &outcome);
            let result = invocation.bind_result(lifecycle, outcome).unwrap();
            preview = preview
                .advance(
                    &engine,
                    &raw,
                    &prepared,
                    HarnessExecutionEvent::RoleResult {
                        result: Box::new(result),
                    },
                )
                .unwrap();
        }
        let evaluated = execute_codex_frontier(
            &engine,
            &prepared,
            head,
            &binary,
            Some(&test_source(&workspace)),
            &RoleObservations::default(),
        )
        .unwrap();
        assert_eq!(evaluated.state, HarnessExecutionState::Evaluated);
        assert_eq!(evaluated.role_execution.role_results.len(), 3);
        assert_eq!(codex_calls(&binary).len(), 3);
        assert_eq!(
            evaluated.role_execution.role_results[0].role,
            HarnessRole::Writer
        );
        let contexts = evaluated
            .role_execution
            .role_results
            .iter()
            .map(|result| &result.context_id)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            contexts.len(),
            3,
            "each required role keeps an independently observed context"
        );
        assert_eq!(
            evaluated.evaluation.as_ref().unwrap().subject_status,
            context_core::harness::SubjectStatus::Accepted
        );
        assert!(evaluated.validation_receipt_digest.is_none());
        assert!(evaluated.finalization_digest.is_none());
        assert_eq!(fs::read(workspace.join("document-0.md")).unwrap(), original);
        codex_assert_exact_transport(&engine, &prepared, &evaluated, &binary);
    }

    #[cfg(unix)]
    #[test]
    fn codex_rejected_reviewer_preserves_result_and_last_accepted_head() {
        use context_core::harness::MissingContextCandidate;
        let temp = TempDirectory::new("codex-rejected-diagnostic");
        let (engine, prepared, raw, workspace) =
            codex_fixture_for_action(&temp, 1, 1, HarnessAction::DocumentWrite);
        let head =
            HarnessExecutionRecord::begin_durable(&engine, &raw, &prepared, "rejected-diagnostic")
                .unwrap();
        let binary = codex_test_script(&temp, CODEX_CAPTURE);
        let mut preview = head.clone();
        let mut number = 1;
        while let Some(invocation) = preview.ready_role_invocations.first() {
            let mut outcome =
                codex_write_scenario_outcome(&prepared, invocation, "reviewer-missing-context");
            if invocation.role == HarnessRole::Reviewer {
                let RoleExecutionOutcome::MissingContext { request } = &mut outcome else {
                    unreachable!()
                };
                request.candidate = MissingContextCandidate::VaultEvidence {
                    repository_relative_path: "vault/profile/index.md".to_owned(),
                };
                codex_test_events(&binary, number, outcome);
                break;
            }
            codex_test_events(&binary, number, outcome.clone());
            number += 1;
            let lifecycle =
                codex_test_lifecycle(invocation.role, format!("preview-{number}"), &outcome);
            let result = invocation.bind_result(lifecycle, outcome).unwrap();
            preview = preview
                .advance(
                    &engine,
                    &raw,
                    &prepared,
                    HarnessExecutionEvent::RoleResult {
                        result: Box::new(result),
                    },
                )
                .unwrap();
        }
        let error = execute_codex_frontier(
            &engine,
            &prepared,
            head,
            &binary,
            None,
            &RoleObservations::default(),
        )
        .unwrap_err();
        let diagnostic: serde_json::Value = serde_json::from_str(&error.diagnostic_json()).unwrap();
        assert_eq!(diagnostic["kind"], "role-submission-error");
        let submitted: RoleExecutionResult =
            serde_json::from_value(diagnostic["submission"]["submitted_result"].clone()).unwrap();
        assert_eq!(submitted.role, HarnessRole::Reviewer);
        assert!(matches!(
            submitted.outcome,
            RoleExecutionOutcome::MissingContext { .. }
        ));
        assert!(!submitted.context_id.is_empty());
        assert_eq!(
            submitted.lifecycle.terminal_state,
            RoleTerminalState::MissingContext
        );
        let stored: HarnessExecutionRecord = read_harness_json(
            &workspace.join(".llm-context-vault-harness/runs/rejected-diagnostic/head.json"),
            "last accepted head",
        )
        .unwrap();
        assert_eq!(stored.sequence, preview.sequence);
        assert_eq!(stored.role_execution.role_results.len(), 2);
        assert!(stored.evaluation.is_none());
        assert!(stored.validation_receipt_digest.is_none());
        assert!(stored.finalization_digest.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn codex_specialist_uses_the_same_bound_producer_transport() {
        use context_core::harness::{EvaluationSubject, SpecialistArtifact};
        let temp = TempDirectory::new("codex-specialist-flow");
        let (engine, prepared, raw, _) =
            codex_fixture_for_action(&temp, 1, 1, HarnessAction::Design);
        let head =
            HarnessExecutionRecord::begin_durable(&engine, &raw, &prepared, "specialist-flow")
                .unwrap();
        let binary = codex_test_script(&temp, CODEX_CAPTURE);
        let mut preview = head.clone();
        let mut number = 1;
        while let Some(invocation) = preview.ready_role_invocations.first() {
            let outcome = if invocation.role == HarnessRole::Specialist {
                let metadata = prepared
                    .role_run
                    .role_metadata
                    .iter()
                    .find(|m| m.role == HarnessRole::Specialist)
                    .unwrap();
                let RoleTaskContract::Specialist {
                    source_targets,
                    verification_requirements,
                } = &metadata.task
                else {
                    unreachable!()
                };
                let evidence = source_targets
                    .iter()
                    .map(|target| {
                        let TargetState::Existing { content_digest } = &target.state else {
                            unreachable!()
                        };
                        ResultEvidenceReference::Target {
                            workspace_relative_path: target.workspace_relative_path.clone(),
                            content_digest: content_digest.clone(),
                            locator: "entire source".into(),
                        }
                    })
                    .collect::<Vec<_>>();
                RoleExecutionOutcome::Completed {
                    result: CompletedRoleResult::Specialist {
                        artifact: SpecialistArtifact {
                            source_targets: source_targets
                                .iter()
                                .map(|t| t.workspace_relative_path.clone())
                                .collect(),
                            context_bundle_digest: None,
                            output: "Synthetic source-grounded design".into(),
                        },
                        requirement_results: verification_requirements
                            .iter()
                            .map(|r| RequirementResult {
                                unit: r.unit,
                                passed: true,
                                detail: "Synthetic check".into(),
                                evidence: evidence.clone(),
                            })
                            .collect(),
                        promotion_proposals: vec![],
                    },
                }
            } else {
                let EvaluationSubject::ProducedArtifact { artifact_digest } = &invocation.subject
                else {
                    unreachable!()
                };
                codex_completed_with_evidence(
                    &prepared,
                    invocation.role,
                    false,
                    vec![ResultEvidenceReference::ProducedArtifact {
                        artifact_digest: artifact_digest.clone(),
                        locator: "entire output".into(),
                    }],
                )
            };
            codex_test_events(&binary, number, outcome.clone());
            number += 1;
            let lifecycle =
                codex_test_lifecycle(invocation.role, format!("preview-{number}"), &outcome);
            let result = invocation.bind_result(lifecycle, outcome).unwrap();
            preview = preview
                .advance(
                    &engine,
                    &raw,
                    &prepared,
                    HarnessExecutionEvent::RoleResult {
                        result: Box::new(result),
                    },
                )
                .unwrap();
        }
        let evaluated = execute_codex_frontier(
            &engine,
            &prepared,
            head,
            &binary,
            None,
            &RoleObservations::default(),
        )
        .unwrap();
        assert_eq!(
            evaluated.role_execution.role_results[0].role,
            HarnessRole::Specialist
        );
        assert_eq!(
            evaluated.evaluation.as_ref().unwrap().subject_status,
            context_core::harness::SubjectStatus::Accepted
        );
        codex_assert_exact_transport(&engine, &prepared, &evaluated, &binary);
    }

    #[cfg(unix)]
    #[test]
    fn codex_tool_backed_writer_is_rejected_before_creating_a_durable_run() {
        let temp = TempDirectory::new("codex-writer-tools");
        let (_, _, raw, workspace) =
            codex_fixture_for_action(&temp, 1, 1, HarnessAction::CodeWrite);
        let binary = codex_test_script(&temp, CODEX_CAPTURE);
        let prepared_path = temp.path().join("prepared.json");
        fs::write(&prepared_path, raw).unwrap();
        let error = run_begin_harness_command([
            "--context-view".to_owned(),
            temp.path().join("source-placeholder").display().to_string(),
            "--workspace-root".to_owned(),
            workspace.display().to_string(),
            "--prepared-run".to_owned(),
            prepared_path.display().to_string(),
            "--run-id".to_owned(),
            "unsupported-writer-tools".to_owned(),
            "--codex-binary".to_owned(),
            binary.display().to_string(),
        ])
        .unwrap_err();
        assert!(error.to_string().contains("tool-free plan"));
        assert!(codex_calls(&binary).is_empty());
        assert!(
            !workspace
                .join(".llm-context-vault-harness/runs/unsupported-writer-tools")
                .exists()
        );
    }

    #[cfg(unix)]
    #[test]
    fn codex_durable_write_terminal_evaluation_preserves_candidate_authority() {
        use context_core::harness::{ExecutionStatus, HarnessCompletionState, SubjectStatus};

        for (scenario, execution, completion) in [
            (
                "writer-failed",
                ExecutionStatus::Failed,
                HarnessCompletionState::ExecutionHalted,
            ),
            (
                "reviewer-missing-context",
                ExecutionStatus::MissingContext,
                HarnessCompletionState::MissingContext,
            ),
            (
                "completed-rejected",
                ExecutionStatus::Completed,
                HarnessCompletionState::RevisionRequired,
            ),
        ] {
            let temp = TempDirectory::new("codex-write-terminal");
            let (engine, prepared, raw, workspace) =
                codex_fixture_for_action(&temp, 2, 8, HarnessAction::DocumentWrite);
            fs::write(
                workspace.join("additional.md"),
                "# Additional synthetic source\n",
            )
            .expect("MissingContext must name an existing unbound document");
            assert!(prepared.plan.source_write_allowed);
            assert!(prepared.accepted_tool_plan.is_none());
            let begun = HarnessExecutionRecord::begin_durable(&engine, &raw, &prepared, scenario)
                .expect("durable document write must begin");
            let terminal = codex_finish_write_scenario(&engine, &prepared, begun, scenario);
            let arguments = [
                "--context-view".to_owned(),
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../..")
                    .to_string_lossy()
                    .into_owned(),
                "--workspace-root".to_owned(),
                workspace.to_string_lossy().into_owned(),
                "--run-id".to_owned(),
                scenario.to_owned(),
            ];
            run_evaluate_harness_command(arguments.clone())
                .expect("durable CLI evaluation must preserve every valid write terminal");
            let directory = workspace
                .join(".llm-context-vault-harness/runs")
                .join(scenario);
            let head: HarnessExecutionRecord =
                read_harness_json(&directory.join("head.json"), "evaluated write head")
                    .expect("evaluated write head must decode strictly");
            head.validate(&prepared, &raw)
                .expect("durable write receipt must validate exactly");
            assert_eq!(head.state, HarnessExecutionState::Evaluated);
            assert!(head.run_evaluation_receipt_digest.is_some());
            assert_eq!(head.role_execution, terminal.role_execution);
            let evaluation = head
                .evaluation
                .as_ref()
                .expect("task evaluation receipt must exist");
            assert_eq!(evaluation.execution_status, execution);
            assert_eq!(evaluation.completion_state, completion);
            let completed = execution == ExecutionStatus::Completed;
            assert_eq!(head.candidate_digest.is_some(), completed);
            assert_eq!(head.candidate_digest, evaluation.candidate_digest);
            assert_eq!(evaluation.artifact.is_some(), completed);
            assert_eq!(evaluation.subject.is_some(), completed);
            assert_eq!(
                evaluation.missing_context.is_some(),
                execution == ExecutionStatus::MissingContext
            );
            if completed {
                assert_eq!(evaluation.subject_status, SubjectStatus::Rejected);
                assert!(
                    evaluation
                        .requirements
                        .iter()
                        .any(|requirement| !requirement.passed)
                );
            } else {
                assert_eq!(evaluation.subject_status, SubjectStatus::NotApplicable);
                assert!(evaluation.requirements.is_empty());
            }
            assert!(run_validate_harness_command(arguments.clone()).is_err());
            assert!(run_apply_harness_command(arguments).is_err());
            let unchanged: HarnessExecutionRecord =
                read_harness_json(&directory.join("head.json"), "unmodified write head")
                    .expect("forbidden validation and apply must leave a valid head");
            assert_eq!(unchanged, head);
            assert!(unchanged.validation_receipt_digest.is_none());
            assert!(unchanged.finalization_digest.is_none());
            assert!(!directory.join("apply-attempt.json").exists());
            prepared
                .plan
                .frozen_targets
                .revalidate_workspace(&workspace)
                .expect("all frozen source documents must remain unchanged");
        }
    }

    #[cfg(unix)]
    #[test]
    fn codex_missing_context_maps_the_existing_request_and_core_halts() {
        use context_core::harness::{
            ExecutionStatus, MissingContextCandidate, MissingContextRequest,
        };
        let temp = TempDirectory::new("codex-missing-context");
        let (engine, prepared, raw, workspace) = codex_fixture(&temp, 2, 8);
        fs::write(
            workspace.join("additional.md"),
            "# Additional synthetic source\n",
        )
        .expect("MissingContext must name an existing unbound workspace source");
        let mut head =
            HarnessExecutionRecord::begin_durable(&engine, &raw, &prepared, "missing-context")
                .expect("run must begin");
        // Submit any ready Verifier first so the MissingContext is issued by Reviewer.
        if let Some(invocation) = head
            .ready_role_invocations
            .iter()
            .find(|invocation| invocation.role == HarnessRole::Verifier)
        {
            let outcome = codex_completed(&prepared, HarnessRole::Verifier, false);
            let result = invocation
                .bind_result(
                    codex_test_lifecycle(invocation.role, "external-verifier".to_owned(), &outcome),
                    outcome,
                )
                .expect("Verifier must bind");
            head = HarnessExecutionRecord::advance_durable(
                &engine,
                "missing-context",
                HarnessExecutionEvent::RoleResult {
                    result: Box::new(result),
                },
            )
            .expect("Verifier must advance");
        }
        let RoleExecutionOutcome::Completed {
            result:
                CompletedRoleResult::Reviewer {
                    subject_evidence,
                    requirement_results,
                    ..
                },
        } = codex_completed(&prepared, HarnessRole::Reviewer, false)
        else {
            panic!("fixture must produce Reviewer fields");
        };
        let outcome = RoleExecutionOutcome::MissingContext {
            request: MissingContextRequest {
                reason: "one additional source is needed".to_owned(),
                blocked_verification_units: vec![requirement_results[0].unit],
                subject_evidence,
                candidate: MissingContextCandidate::AdditionalWorkspaceTarget {
                    workspace_relative_path: "additional.md".to_owned(),
                },
            },
        };
        let binary = codex_test_script(&temp, CODEX_CAPTURE);
        codex_test_events(&binary, 1, outcome.clone());
        let evaluated = execute_codex_frontier(
            &engine,
            &prepared,
            head,
            &binary,
            None,
            &RoleObservations::default(),
        )
        .expect("Core must accept the existing MissingContext shape");
        assert_eq!(
            evaluated
                .evaluation
                .expect("Core must evaluate")
                .execution_status,
            ExecutionStatus::MissingContext
        );
        assert_eq!(codex_calls(&binary).len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn codex_timeout_bounds_blocked_stdin_and_reaps_the_process_group() {
        use std::os::unix::fs::PermissionsExt as _;
        use std::time::{Duration, Instant};
        let temp = TempDirectory::new("codex-timeout");
        // This test needs an immediate context event and blocked input. Avoid
        // the general fixture's external ls/cp startup before that event.
        let binary = temp.path().join("fake-codex");
        fs::write(
            &binary,
            r#"#!/bin/sh
trap '' INT TERM
printf '%s\n' "$$" >> "$0.calls"
printf '%s\n' "$PWD" > "$0.cwd.$$"
printf '{"type":"thread.started","thread_id":"native-%s"}\n' "$$"
sleep 30 &
printf '%s\n' "$!" > "$0.descendant"
wait
"#,
        )
        .expect("blocked-input fixture must be written");
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700))
            .expect("blocked-input fixture must be executable");
        let limits = RoleLifecycleLimits {
            max_role_execution_millis: 3_000,
            max_role_grace_millis: 300,
            max_role_close_millis: 1_000,
            max_total_role_millis: 4_300,
        };
        let started = Instant::now();
        let (lifecycle, outcome) = run_codex_process(
            &binary,
            &vec![b'x'; 1024 * 1024],
            HarnessRole::Reviewer,
            &limits,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .expect("blocked stdin must terminate as an observed timeout");
        assert_eq!(outcome, RoleExecutionOutcome::TimedOut);
        assert!(lifecycle.context_id.starts_with("native-"));
        assert!(lifecycle.context_ready_at_millis >= lifecycle.started_at_millis);
        assert!(
            lifecycle.context_ready_at_millis
                <= lifecycle
                    .interrupt_requested_at_millis
                    .expect("timeout must observe an interrupt")
        );
        assert!(lifecycle.interrupt_requested_at_millis.is_some());
        assert!(
            lifecycle.terminal_at_millis
                <= lifecycle
                    .grace_deadline_at_millis
                    .expect("timeout must record its grace deadline")
        );
        assert!(
            lifecycle.closed_at_millis - lifecycle.started_at_millis
                <= limits.max_total_role_millis
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        let calls = codex_calls(&binary);
        assert_eq!(calls.len(), 1, "timeout must not retry");
        let pid: i32 = calls[0].parse().expect("captured PID must parse");
        // SAFETY: signal zero only probes the synthetic process; it cannot signal it.
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            -1,
            "leader must be reaped before returning"
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
        // EOF on the pipes retained by `sleep` additionally proves group shutdown.
        let cwd = fs::read_to_string(format!("{}.cwd.{pid}", binary.display()))
            .expect("cwd must be captured");
        assert!(!Path::new(cwd.lines().next().expect("cwd must have path")).exists());
        let mut total = MAX_HARNESS_JSON_BYTES - 1;
        assert!(codex_read_chunk(&mut std::io::repeat(b'x'), &mut [0; 2], &mut total).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn codex_malformed_or_missing_responses_and_nonzero_exit_halt_in_core() {
        use context_core::harness::ExecutionStatus;
        for scenario in ["malformed", "missing", "exit", "native"] {
            let temp = TempDirectory::new("codex-response-failure");
            let (engine, prepared, raw, _) = codex_fixture(&temp, 2, 8);
            let body = if scenario == "exit" {
                format!("{CODEX_CAPTURE}\nexit 7\n")
            } else {
                CODEX_CAPTURE.to_owned()
            };
            let binary = codex_test_script(&temp, &body);
            let head =
                HarnessExecutionRecord::begin_durable(&engine, &raw, &prepared, "bad-response")
                    .expect("run must begin");
            let invocation = &head.ready_role_invocations[0];
            codex_test_events(
                &binary,
                1,
                codex_completed(&prepared, invocation.role, false),
            );
            let events_path = PathBuf::from(format!("{}.events.1", binary.display()));
            if scenario != "exit" {
                let mut events = vec![CodexEvent::TurnStarted];
                if scenario == "malformed" {
                    events.push(CodexEvent::ItemCompleted {
                        item: CodexItem::AgentMessage {
                            id: "message".to_owned(),
                            text: "No Findings".to_owned(),
                        },
                    });
                }
                events.push(CodexEvent::TurnCompleted {
                    usage: codex_test_usage(),
                });
                let text = if scenario == "native" {
                    "malformed native JSONL\n".to_owned()
                } else {
                    codex_test_jsonl(&events)
                };
                fs::write(&events_path, text).expect("failure fixture must be written");
            }
            let evaluated = execute_codex_frontier(
                &engine,
                &prepared,
                head,
                &binary,
                None,
                &RoleObservations::default(),
            )
            .expect("observed failure must reach Core evaluation");
            assert_eq!(
                evaluated
                    .evaluation
                    .expect("Core must evaluate")
                    .execution_status,
                ExecutionStatus::Failed
            );
            assert_eq!(evaluated.role_execution.role_results.len(), 1);
            assert_eq!(
                codex_calls(&binary).len(),
                1,
                "no retry or sibling launch after {scenario}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn codex_completed_output_cannot_hide_a_stuck_process_or_inherited_pipes() {
        let temp = TempDirectory::new("codex-close-timeout");
        let body = format!("{CODEX_CAPTURE}\ntrap '' INT TERM\nsleep 30 &\nwait\n");
        let binary = codex_test_script(&temp, &body);
        codex_test_events(&binary, 1, RoleExecutionOutcome::Cancelled);
        let limits = RoleLifecycleLimits {
            max_role_execution_millis: 2_000,
            max_role_grace_millis: 100,
            max_role_close_millis: 500,
            max_total_role_millis: 2_600,
        };
        let (lifecycle, outcome) = run_codex_process(
            &binary,
            b"[]",
            HarnessRole::Reviewer,
            &limits,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .expect("stuck closing process must be terminated and reaped");
        assert_eq!(outcome, RoleExecutionOutcome::TimedOut);
        assert!(lifecycle.first_output_at_millis.is_some());
        assert!(
            lifecycle.closed_at_millis - lifecycle.terminal_at_millis
                <= limits.max_role_close_millis
        );
        assert_eq!(codex_calls(&binary).len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn codex_input_cap_and_invalid_binary_never_launch_a_process() {
        let temp = TempDirectory::new("codex-input-cap");
        let (engine, mut prepared, raw, _) = codex_fixture(&temp, 2, 8);
        let binary = codex_test_script(&temp, CODEX_CAPTURE);
        let head = HarnessExecutionRecord::begin_durable(&engine, &raw, &prepared, "input-cap")
            .expect("run must begin");
        prepared
            .role_run
            .runtime_capabilities
            .max_role_invocation_bytes = 1;
        assert!(
            execute_codex_role(
                &binary,
                &prepared,
                &head.ready_role_invocations[0],
                &std::sync::atomic::AtomicBool::new(false),
                &RoleObservations::default(),
            )
            .expect_err("over-limit input must be unsupported")
            .to_string()
            .contains("input limit")
        );
        assert!(codex_calls(&binary).is_empty());
        assert!(validate_codex_binary(temp.path()).is_err());
        assert!(validate_codex_binary(&temp.path().join("missing-binary")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn codex_advance_without_binary_remains_json_only() {
        let temp = TempDirectory::new("codex-json-only-advance");
        let (engine, prepared, raw, workspace) = codex_fixture(&temp, 2, 8);
        let head = HarnessExecutionRecord::begin_durable(&engine, &raw, &prepared, "json-only")
            .expect("run must begin");
        let invocation = &head.ready_role_invocations[0];
        let outcome = codex_completed(&prepared, invocation.role, false);
        let result = invocation
            .bind_result(
                codex_test_lifecycle(invocation.role, "external-json-only".to_owned(), &outcome),
                outcome,
            )
            .expect("external result must bind");
        let event_path = temp.path().join("event.json");
        write_test_json(
            &event_path,
            &HarnessExecutionEvent::RoleResult {
                result: Box::new(result),
            },
        );
        let repository = temp.path().join("source-placeholder");
        run_advance_harness_command([
            "--context-view".to_owned(),
            repository.to_string_lossy().into_owned(),
            "--workspace-root".to_owned(),
            workspace.to_string_lossy().into_owned(),
            "--run-id".to_owned(),
            "json-only".to_owned(),
            "--event".to_owned(),
            event_path.to_string_lossy().into_owned(),
            "--json".to_owned(),
        ])
        .expect("advance without Codex must preserve its existing behavior");
        let head: HarnessExecutionRecord = read_harness_json(
            &workspace.join(".llm-context-vault-harness/runs/json-only/head.json"),
            "JSON-only head",
        )
        .expect("head must decode");
        assert_eq!(head.state, HarnessExecutionState::Executing);
        assert_eq!(head.role_execution.role_results.len(), 1);
        assert_eq!(head.ready_role_invocations.len(), 1);
        assert!(head.evaluation.is_none());
    }

    fn test_serialized_digest(value: &impl serde::Serialize) -> String {
        let bytes = serde_json::to_vec(value).expect("test value must serialize");
        test_byte_digest(&bytes)
    }

    fn test_byte_digest(bytes: &[u8]) -> String {
        let digest = Sha256::digest(bytes);
        let mut encoded = String::with_capacity(digest.len() * 2);
        for byte in digest {
            write!(&mut encoded, "{byte:02x}")
                .expect("writing a SHA-256 digest to a String must succeed");
        }
        encoded
    }

    fn test_configured_control_digest() -> String {
        let store = test_store();
        test_runtime().block_on(async {
            sqlx::query_scalar("SELECT content_digest FROM context_materials WHERE scope='profile' AND path='rules/control.md'").fetch_one(store.pool()).await.expect("fixture policy metadata")
        })
    }

    #[test]
    fn harness_input_requires_request_envelope() {
        let error = harness_input(
            [
                "--context-view".to_owned(),
                "/vault-repository".to_owned(),
                "--workspace-root".to_owned(),
                "/workspace".to_owned(),
            ],
            "resolve",
        )
        .expect_err("missing request envelope must be rejected");

        assert!(error.to_string().contains("missing `--request-envelope`"));
    }

    #[test]
    fn harness_input_parses_confirmed_curation() {
        let temp = TempDirectory::new("harness-curation");
        let envelope_path = write_user_request_envelope(
            &temp,
            "curation",
            HarnessRequest {
                action: HarnessAction::VaultCuration,
                owner: DataOwner::Personal,
                targets: vec!["vault/personal/knowledge/item.md".to_owned()],
                objective: "save knowledge".to_owned(),
                curation_kind: Some(CurationKind::Knowledge),
                explicit_user_confirmation_reported: true,
                curation_sources: vec!["vault/personal/index.md".to_owned()],
                delete_targets: Vec::new(),
            },
            HarnessIntent::General,
            Vec::new(),
            Vec::new(),
            HarnessExecutionProfile::Standard,
        );
        let input = harness_input(
            [
                "--context-view".to_owned(),
                "/vault-repository".to_owned(),
                "--workspace-root".to_owned(),
                "/vault-repository".to_owned(),
                "--request-envelope".to_owned(),
                envelope_path.to_string_lossy().into_owned(),
            ],
            "resolve",
        )
        .expect("confirmed curation input must parse");

        let (contract, _, _) = input
            .request_envelope
            .resolve()
            .expect("curation request envelope must resolve");
        let ResolvedTaskContract::Curation(contract) = contract else {
            panic!("request must resolve to curation");
        };
        assert_eq!(contract.curation_kind, CurationKind::Knowledge);
        assert_eq!(contract.confirmation, UserConfirmationStatus::Confirmed);
        assert_eq!(contract.curation_sources, ["vault/personal/index.md"]);
    }

    fn test_runtime() -> &'static tokio::runtime::Runtime {
        static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
        RT.get_or_init(|| tokio::runtime::Runtime::new().expect("native fixture runtime"))
    }
    fn test_store() -> &'static Store {
        static STORE: std::sync::OnceLock<Store> = std::sync::OnceLock::new();
        STORE.get_or_init(|| {
            test_runtime().block_on(async {
                let url =
                    std::env::var("TEST_DATABASE_URL").expect("isolated native test database");
                let options = crate::config::database_options(&url).expect("local test URL");
                assert!(
                    options
                        .get_database()
                        .is_some_and(|name| name.starts_with("ontology_test_"))
                );
                let store = Store::connect(&url).await.expect("test database");
                store.initialize().await.expect("test migrations");
                sqlx::query("TRUNCATE document_grouping,document_subjects,context_source_bindings,context_projection_versions,context_material_versions,context_manual_edits,context_materials,context_apply_batches").execute(store.pool()).await.expect("reset owned native CLI fixture");
                let fixture = context_fixture::build();
                let root = fixture.path().canonicalize().expect("synthetic fixture root");
                let scopes = context_fixture::SCOPES
                    .iter()
                    .map(|s| s.parse().expect("declared synthetic scope"))
                    .collect::<Vec<_>>();
                let inventory = crate::context::inventory(&root, &scopes)
                    .expect("synthetic fixture inventory");
                assert_eq!(inventory.entries.len(), context_fixture::DOCUMENT_COUNT);
                assert_eq!(
                    inventory.total_bytes,
                    context_fixture::TOTAL_BYTES
                );
                let mut bindings = String::new();
                for (path, expected) in context_fixture::documents() {
                    let bytes = fs::read(root.join(path)).expect("declared synthetic fixture file");
                    assert_eq!(bytes, expected.as_bytes(), "declared bytes for {path}");
                    bindings.push_str(&format!(
                        "{path}\t{}\t{}\n",
                        bytes.len(),
                        test_byte_digest(&bytes)
                    ));
                }
                assert_eq!(
                    test_byte_digest(bindings.as_bytes()),
                    context_fixture::CONTRACT_DIGEST,
                    "declared fixture paths, lengths, and content digests"
                );
                store
                    .import_context(&root, &scopes, &inventory.inventory_digest)
                    .await
                    .expect("native fixture import");
                store
            })
        })
    }
    fn test_sources() -> &'static std::sync::Mutex<BTreeMap<PathBuf, Arc<NativeContextSource>>> {
        static SOURCES: std::sync::OnceLock<
            std::sync::Mutex<BTreeMap<PathBuf, Arc<NativeContextSource>>>,
        > = std::sync::OnceLock::new();
        SOURCES.get_or_init(Default::default)
    }
    fn test_source(workspace: &Path) -> Arc<NativeContextSource> {
        let workspace = workspace.canonicalize().expect("test workspace");
        let mut sources = test_sources().lock().expect("test sources lock");
        if let Some(source) = sources.get(&workspace) {
            return source.clone();
        }
        let view = workspace.join(".native-test-view");
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&view)
            .expect("owned native test view");
        let store = test_store();
        let source = test_runtime()
            .block_on(store.with_native_context(view, Ok))
            .expect("native source");
        sources.insert(workspace, source.clone());
        source
    }
    fn test_engine(
        _root: impl AsRef<Path>,
        workspace: impl AsRef<Path>,
        router: HarnessRouter,
    ) -> std::result::Result<HarnessEngine, HarnessError> {
        let source = test_source(workspace.as_ref());
        HarnessEngine::with_source_and_router(
            source.view_root(),
            workspace,
            source.clone(),
            router,
            context_fixture::configuration(),
        )
    }
    fn test_engine_default(
        root: impl AsRef<Path>,
        workspace: impl AsRef<Path>,
    ) -> std::result::Result<HarnessEngine, HarnessError> {
        test_engine(
            root,
            workspace,
            HarnessRouter::with_execution_profile(2, HarnessExecutionProfile::Standard)?,
        )
    }
    fn run(arguments: impl IntoIterator<Item = String>) -> Result<()> {
        let mut args = arguments.into_iter();
        let _program = args.next();
        assert_eq!(args.next().as_deref(), Some("harness"));
        run_harness_command(args)
    }
    fn run_harness_command(arguments: impl IntoIterator<Item = String>) -> Result<()> {
        let mut args = arguments.into_iter().collect::<Vec<_>>();
        let command = args.first().cloned().unwrap_or_default();
        let workspace = args
            .windows(2)
            .find(|w| w[0] == "--workspace-root")
            .map(|w| PathBuf::from(&w[1]));
        if workspace.as_ref().is_none_or(|p| !p.is_dir()) {
            return test_runtime().block_on(super::run(args));
        }
        let workspace = workspace.expect("checked workspace");
        let source = test_source(&workspace);
        for i in 0..args.len().saturating_sub(1) {
            if args[i] == "--context-view" {
                args[i + 1] = source.view_root().to_string_lossy().into_owned();
            }
        }
        if matches!(command.as_str(), "apply" | "recover") {
            args.extend([
                "--policy-config".into(),
                context_fixture::configuration_path().display().to_string(),
                "--store-id".into(),
                source
                    .store_identity()
                    .expect("identity")
                    .expect("native")
                    .store_id,
            ]);
            return test_runtime().block_on(super::run(args));
        }
        Command {
            source,
            policy_configuration: context_fixture::configuration(),
        }
        .run_harness_command(args.into_iter(), &RoleObservations::default())
        .and_then(print_output)
    }
    fn run_begin_harness_command(arguments: impl IntoIterator<Item = String>) -> Result<()> {
        run_harness_command(std::iter::once("begin".to_owned()).chain(arguments))
    }
    fn run_advance_harness_command(arguments: impl IntoIterator<Item = String>) -> Result<()> {
        run_harness_command(std::iter::once("advance".to_owned()).chain(arguments))
    }
    fn run_evaluate_harness_command(arguments: impl IntoIterator<Item = String>) -> Result<()> {
        run_harness_command(std::iter::once("evaluate".to_owned()).chain(arguments))
    }
    fn run_validate_harness_command(arguments: impl IntoIterator<Item = String>) -> Result<()> {
        run_harness_command(std::iter::once("validate".to_owned()).chain(arguments))
    }
    fn run_apply_harness_command(arguments: impl IntoIterator<Item = String>) -> Result<()> {
        run_harness_command(std::iter::once("apply".to_owned()).chain(arguments))
    }
}
