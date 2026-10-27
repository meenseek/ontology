use crate::native_harness::{
    MAX_HARNESS_JSON_BYTES, NativeHarnessError, NativeHarnessResult as Result,
};
use context_core::harness::{
    FrozenReviewArtifact, HarnessEngine, HarnessExecutionEvent, HarnessExecutionRecord,
    HarnessRole, PreparedHarnessRun, ReportedRoleLifecycle, ReviewArtifactFormat,
    RoleExecutionOutcome, RoleExecutionResult, RoleInvocationContract, RoleLifecycleLimits,
    RoleTaskContract, RoleTerminalState, VerificationRequirement, decode_current_json,
};
use std::{
    env, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicBool, Ordering},
};
// This adapter only transports Core-issued segments and observes a native process.
// It does not select policies, evaluate evidence, or authorize source mutations.
pub(crate) fn validate_codex_binary(binary: &Path) -> Result<()> {
    if !binary.is_absolute() {
        return Err(NativeHarnessError::invalid_input(
            "--codex-binary requires an absolute executable path",
        ));
    }
    let metadata = fs::metadata(binary)
        .map_err(|error| NativeHarnessError::io("inspect Codex executable", binary, error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 {
            return Ok(());
        }
    }
    let _ = metadata;
    Err(NativeHarnessError::invalid_input(
        "unsupported runtime: Codex execution requires a Unix executable file",
    ))
}

pub(crate) fn execute_codex_frontier(
    engine: &HarnessEngine,
    prepared: &PreparedHarnessRun,
    mut record: HarnessExecutionRecord,
    binary: &Path,
    source: Option<&crate::native_context::NativeContextSource>,
    observations: &RoleObservations,
) -> Result<HarnessExecutionRecord> {
    // Only callers holding the typed return of begin/advance/revise_durable enter
    // here. Never deserialize a caller-supplied frontier or replay a stale sibling.
    let mut executed = std::collections::BTreeSet::new();
    loop {
        if record.prepared_run_digest != prepared.prepared_run_digest {
            return Err(NativeHarnessError::invalid_input(
                "Core frontier and Codex lifecycle preparation differ",
            ));
        }
        if record.ready_tool_invocation.is_some() {
            return Err(NativeHarnessError::invalid_input(
                "unsupported runtime: a ready tool invocation requires its existing tool executor",
            ));
        }
        validate_codex_producer_frontier(
            prepared,
            record
                .ready_role_invocations
                .iter()
                .map(|invocation| invocation.role),
        )?;
        if record.ready_role_invocations.is_empty() {
            // A non-completed result also empties the frontier. Core owns the
            // terminal evaluation and may reject evaluation of an incomplete write.
            if prepared.accepted_tool_plan.is_some() {
                return Ok(record);
            }
            return HarnessExecutionRecord::evaluate_durable(engine, &record.run_identifier, None)
                .map_err(|error| NativeHarnessError::invalid_input(error.to_string()));
        }
        let invocations = record
            .ready_role_invocations
            .iter()
            .take(
                prepared
                    .role_run
                    .runtime_capabilities
                    .max_concurrent_roles
                    .min(2),
            )
            .cloned()
            .collect::<Vec<_>>();
        if invocations.is_empty() {
            return Err(NativeHarnessError::invalid_input(
                "unsupported runtime: no concurrent role capacity",
            ));
        }
        for invocation in &invocations {
            if !executed.insert(invocation.role) {
                return Err(NativeHarnessError::invalid_input(
                    "unsupported runtime: automatic role retries are not supported",
                ));
            }
        }
        let artifacts = engine
            .freeze_review_artifacts(
                &prepared.plan.resolved_request,
                &prepared.role_run,
                &invocations,
            )
            .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
        let artifacts = artifacts.as_slice();
        let cancelled = AtomicBool::new(false);
        if let Some(source) = source {
            source
                .suspend_reads()
                .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
        }
        let outcome = std::thread::scope(|scope| {
            let (sender, receiver) = std::sync::mpsc::channel();
            let cancelled = &cancelled;
            let outcome = (|| {
                for invocation in &invocations {
                    let sender = sender.clone();
                    std::thread::Builder::new()
                        .spawn_scoped(scope, move || {
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    execute_codex_role(
                                        binary,
                                        prepared,
                                        invocation,
                                        artifacts,
                                        cancelled,
                                        observations,
                                    )
                                }))
                                .unwrap_or_else(|_| {
                                    Err(NativeHarnessError::invalid_input(
                                        "Codex role worker panicked",
                                    ))
                                });
                            let _ = sender.send(result);
                        })
                        .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
                }
                drop(sender);
                // Only the parent submits, in observed completion order, against
                // Core's current durable head. Independent siblings keep their input.
                for result in receiver {
                    let result = result?;
                    let halted = !matches!(result.outcome, RoleExecutionOutcome::Completed { .. });
                    let submitted = result.clone();
                    if let Some(source) = source {
                        source.resume_reads().map_err(|error| {
                            NativeHarnessError::submission_error(
                                error,
                                &record.run_identifier,
                                submitted.clone(),
                            )
                        })?;
                    }
                    record = HarnessExecutionRecord::advance_durable(
                        engine,
                        &record.run_identifier,
                        HarnessExecutionEvent::RoleResult {
                            result: Box::new(result),
                        },
                    )
                    .map_err(|error| {
                        NativeHarnessError::submission_error(
                            error,
                            &record.run_identifier,
                            submitted,
                        )
                    })?;
                    if let Some(source) = source {
                        source.suspend_reads().map_err(|error| {
                            NativeHarnessError::invalid_input(error.to_string())
                        })?;
                    }
                    if halted {
                        break;
                    }
                }
                Ok(())
            })();
            // A failed/terminal submission closes running siblings before scope
            // joins them. Their results are never submitted after the halt.
            cancelled.store(true, Ordering::Relaxed);
            // Join cancelled siblings without holding the store gate, including
            // when reacquisition/submission failed with the gate held.
            if let Some(source) = source {
                let released = source.suspend_reads();
                if outcome.is_ok() {
                    released
                        .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
                }
            }
            outcome
        });
        let restored = source.map(|source| source.resume_reads()).transpose();
        // Retain the original rejected result diagnostic even if restoration fails.
        outcome?;
        restored.map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
    }
}

pub(crate) fn validate_codex_producer_frontier(
    prepared: &PreparedHarnessRun,
    roles: impl IntoIterator<Item = HarnessRole>,
) -> Result<()> {
    if prepared.accepted_tool_plan.is_some()
        && roles
            .into_iter()
            .any(|role| matches!(role, HarnessRole::Writer | HarnessRole::Specialist))
    {
        return Err(NativeHarnessError::invalid_input(
            "unsupported runtime: automatic producers require a tool-free plan; use the existing explicit tool execution flow",
        ));
    }
    Ok(())
}

// The native output schema needs an object at its root. This transport envelope
// contains exactly the existing outcome; the entire envelope is decoded strictly.
#[derive(serde::Deserialize, serde::Serialize)]
pub(crate) struct CodexResponse {
    pub(crate) outcome: RoleExecutionOutcome,
}

// Formatting constraints only; Core's existing outcome type owns acceptance.
pub(crate) fn codex_output_schema() -> &'static str {
    r##"{
  "type": "object",
  "properties": {"outcome":{"anyOf":[{"type":"object","properties":{"status":{"type":"string","enum":["completed"]},"result":{"$ref":"#/$defs/result"}},"required":["status","result"],"additionalProperties":false},{"type":"object","properties":{"status":{"type":"string","enum":["missing-context"]},"request":{"$ref":"#/$defs/request"}},"required":["status","request"],"additionalProperties":false},{"type":"object","properties":{"status":{"type":"string","enum":["failed"]},"message":{"type":"string"}},"required":["status","message"],"additionalProperties":false},{"type":"object","properties":{"status":{"type":"string","enum":["cancelled"]}},"required":["status"],"additionalProperties":false},{"type":"object","properties":{"status":{"type":"string","enum":["timed-out"]}},"required":["status"],"additionalProperties":false},{"type":"object","properties":{"status":{"type":"string","enum":["unsupported"]},"reason":{"type":"string"}},"required":["status","reason"],"additionalProperties":false}]}},
  "required": ["outcome"],
  "additionalProperties": false,
  "$defs": {
    "verification-unit": {"type":"string","enum":["completion-contract","scope-compliance","terminology-and-readability","code-correctness","tests-and-static-analysis","source-ownership","fact-and-claim-boundary","evidence-and-uncertainty","idea-not-promoted-to-decision","solo-mvp-ideation-contract","retrieval-scope-and-digest","curation-provenance","source-and-ontology-boundary","career-evidence-lineage","career-surface-contract","career-perspective-routing","career-public-safety","career-output-surface-selection","resume-first-screen-and-artifact","career-description-case-structure","portfolio-local-build-and-public-copy","professional-profile-artifact","career-holistic-coherence","output-adapter-contract","output-adapter-surface-selection","resume-output-adapter-artifact","career-description-output-adapter-artifact","professional-profile-output-adapter-artifact"]},
    "evidence": {"anyOf":[{"type":"object","properties":{"source":{"type":"string","enum":["target"]},"workspace_relative_path":{"type":"string"},"content_digest":{"type":"string"},"locator":{"type":"string"}},"required":["source","workspace_relative_path","content_digest","locator"],"additionalProperties":false},{"type":"object","properties":{"source":{"type":"string","enum":["bound-document","bound-review-artifact"]},"relative_path":{"type":"string"},"content_digest":{"type":"string"},"locator":{"type":"string"}},"required":["source","relative_path","content_digest","locator"],"additionalProperties":false},{"type":"object","properties":{"source":{"type":"string","enum":["tool-result"]},"unit":{"$ref":"#/$defs/verification-unit"},"result_digest":{"type":"string"}},"required":["source","unit","result_digest"],"additionalProperties":false},{"type":"object","properties":{"source":{"type":"string","enum":["produced-artifact"]},"artifact_digest":{"type":"string"},"locator":{"type":"string"}},"required":["source","artifact_digest","locator"],"additionalProperties":false}]},
    "requirement": {"type":"object","properties":{"unit":{"$ref":"#/$defs/verification-unit"},"passed":{"type":"boolean"},"detail":{"type":"string"},"evidence":{"type":"array","items":{"$ref":"#/$defs/evidence"}}},"required":["unit","passed","detail","evidence"],"additionalProperties":false},
    "observation": {"type":"object","properties":{"message":{"type":"string"},"evidence":{"type":"array","items":{"$ref":"#/$defs/evidence"}}},"required":["message","evidence"],"additionalProperties":false},
    "learning": {"type":"object","properties":{"title":{"type":"string"},"guidance":{"type":"string"},"failed_unit":{"$ref":"#/$defs/verification-unit"},"requirement_result_digest":{"type":"string"},"evidence":{"type":"array","items":{"$ref":"#/$defs/evidence"}}},"required":["title","guidance","failed_unit","requirement_result_digest","evidence"],"additionalProperties":false},
    "result": {"anyOf":[{"type":"object","properties":{"role":{"type":"string","enum":["writer"]},"artifact":{"$ref":"#/$defs/writer-artifact"}},"required":["role","artifact"],"additionalProperties":false},{"type":"object","properties":{"role":{"type":"string","enum":["specialist"]},"artifact":{"type":"object","properties":{"source_targets":{"type":"array","items":{"type":"string"}},"context_bundle_digest":{"type":["string","null"]},"output":{"type":"string"}},"required":["source_targets","context_bundle_digest","output"],"additionalProperties":false},"requirement_results":{"type":"array","items":{"$ref":"#/$defs/requirement"}},"promotion_proposals":{"type":"array","items":{"$ref":"#/$defs/promotion"}}},"required":["role","artifact","requirement_results","promotion_proposals"],"additionalProperties":false},{"type":"object","properties":{"role":{"type":"string","enum":["verifier"]},"subject_evidence":{"type":"array","items":{"$ref":"#/$defs/evidence"}},"requirement_results":{"type":"array","items":{"$ref":"#/$defs/requirement"}}},"required":["role","subject_evidence","requirement_results"],"additionalProperties":false},{"type":"object","properties":{"role":{"type":"string","enum":["reviewer"]},"summary":{"type":"string"},"subject_evidence":{"type":"array","items":{"$ref":"#/$defs/evidence"}},"requirement_results":{"type":"array","items":{"$ref":"#/$defs/requirement"}},"blocking_findings":{"type":"array","items":{"$ref":"#/$defs/observation"}},"improvements":{"type":"array","items":{"$ref":"#/$defs/observation"}},"learning_candidates":{"type":"array","items":{"$ref":"#/$defs/learning"}}},"required":["role","summary","subject_evidence","requirement_results","blocking_findings","improvements","learning_candidates"],"additionalProperties":false}]},
    "request": {"type":"object","properties":{"reason":{"type":"string"},"blocked_verification_units":{"type":"array","items":{"$ref":"#/$defs/verification-unit"}},"subject_evidence":{"type":"array","items":{"$ref":"#/$defs/evidence"}},"candidate":{"anyOf":[{"type":"object","properties":{"kind":{"type":"string","enum":["additional-workspace-target"]},"workspace_relative_path":{"type":"string"}},"required":["kind","workspace_relative_path"],"additionalProperties":false},{"type":"object","properties":{"kind":{"type":"string","enum":["vault-evidence"]},"repository_relative_path":{"type":"string"}},"required":["kind","repository_relative_path"],"additionalProperties":false}]}},"required":["reason","blocked_verification_units","subject_evidence","candidate"],"additionalProperties":false},
    "change": {"anyOf":[{"type":"object","properties":{"operation":{"type":"string","enum":["create"]},"path":{"type":"string"},"content":{"type":"string"}},"required":["operation","path","content"],"additionalProperties":false},{"type":"object","properties":{"operation":{"type":"string","enum":["update"]},"path":{"type":"string"},"expected_content_digest":{"type":"string"},"content":{"type":"string"}},"required":["operation","path","expected_content_digest","content"],"additionalProperties":false},{"type":"object","properties":{"operation":{"type":"string","enum":["delete"]},"path":{"type":"string"},"expected_content_digest":{"type":"string"}},"required":["operation","path","expected_content_digest"],"additionalProperties":false}]},
    "curation-kind": {"type":"string","enum":["idea","knowledge","fact","decision","journal","ontology"]},
    "source-binding": {"type":"object","properties":{"repository_relative_path":{"type":"string"},"content_digest":{"type":"string"}},"required":["repository_relative_path","content_digest"],"additionalProperties":false},
    "curation-entry": {"type":"object","properties":{"change":{"$ref":"#/$defs/change"},"provenance":{"type":"array","items":{"type":"string"}},"source_references":{"type":"array","items":{"$ref":"#/$defs/source-binding"}},"promotion_handoff_digest":{"type":["string","null"]}},"required":["change","provenance","source_references","promotion_handoff_digest"],"additionalProperties":false},
    "writer-artifact": {"anyOf":[{"type":"object","properties":{"kind":{"type":"string","enum":["changes"]},"changes":{"type":"array","items":{"$ref":"#/$defs/change"}}},"required":["kind","changes"],"additionalProperties":false},{"type":"object","properties":{"kind":{"type":"string","enum":["curation"]},"curation_kind":{"$ref":"#/$defs/curation-kind"},"entries":{"type":"array","items":{"$ref":"#/$defs/curation-entry"}}},"required":["kind","curation_kind","entries"],"additionalProperties":false}]},
    "owner": {"anyOf":[{"type":"object","properties":{"kind":{"type":"string","enum":["profile","common-work","personal-business","personal"]}},"required":["kind"],"additionalProperties":false},{"type":"object","properties":{"kind":{"type":"string","enum":["personal-project"]},"project":{"type":"string"}},"required":["kind","project"],"additionalProperties":false},{"type":"object","properties":{"kind":{"type":"string","enum":["company"]},"company":{"type":"string"}},"required":["kind","company"],"additionalProperties":false},{"type":"object","properties":{"kind":{"type":"string","enum":["company-project"]},"company":{"type":"string"},"project":{"type":"string"}},"required":["kind","company","project"],"additionalProperties":false}]},
    "intent": {"anyOf":[{"type":"object","properties":{"kind":{"type":"string","enum":["general","policy-maintenance","solo-mvp-ideation"]}},"required":["kind"],"additionalProperties":false},{"type":"object","properties":{"kind":{"type":"string","enum":["career-artifact","output-adapter"]},"surface":{"type":"string","enum":["general","resume","career-description","portfolio","professional-profile","application-essay","interview"]}},"required":["kind","surface"],"additionalProperties":false}]},
    "promotion-evidence": {"type":"object","properties":{"source_kind":{"type":"string","enum":["target","granted-evidence","retrieval"]},"source_owner":{"$ref":"#/$defs/owner"},"relative_path":{"type":"string"},"content_digest":{"type":"string"},"locator":{"type":"string"}},"required":["source_kind","source_owner","relative_path","content_digest","locator"],"additionalProperties":false},
    "promotion-origin": {"anyOf":[{"type":"object","properties":{"kind":{"type":"string","enum":["specialist-memory"]},"claim_kind":{"type":"string","enum":["observed-fact","model-interpretation","model-proposal"]},"evidence":{"type":"array","items":{"$ref":"#/$defs/promotion-evidence"}}},"required":["kind","claim_kind","evidence"],"additionalProperties":false},{"type":"object","properties":{"kind":{"type":"string","enum":["reviewer-learning"]},"learning_id":{"type":"string"},"source_action":{"type":"string","enum":["code-write","code-review","document-write","document-review","investigation","design","ideation","vault-read","vault-curation"]},"source_intent":{"$ref":"#/$defs/intent"},"failed_unit":{"$ref":"#/$defs/verification-unit"},"requirement_result_digest":{"type":"string"},"evidence":{"type":"array","items":{"$ref":"#/$defs/evidence"}}},"required":["kind","learning_id","source_action","source_intent","failed_unit","requirement_result_digest","evidence"],"additionalProperties":false}]},
    "promotion": {"type":"object","properties":{"identifier":{"type":"string"},"owner":{"$ref":"#/$defs/owner"},"curation_kind":{"$ref":"#/$defs/curation-kind"},"title":{"type":"string"},"content":{"type":"string"},"origin":{"$ref":"#/$defs/promotion-origin"}},"required":["identifier","owner","curation_kind","title","content","origin"],"additionalProperties":false}
  }
}"##
}

// Narrow formatting to Core's already assigned task. This never changes the
// assigned requirements, role outcome, evidence, or Core's acceptance checks.
pub(crate) fn codex_role_output_schema(task: &RoleTaskContract) -> Result<String> {
    let (role, requirements): (&str, &[VerificationRequirement]) = match task {
        RoleTaskContract::Writer { .. } => ("writer", &[]),
        RoleTaskContract::Specialist {
            verification_requirements,
            ..
        } => ("specialist", verification_requirements),
        RoleTaskContract::Verifier {
            verification_requirements,
            ..
        } => ("verifier", verification_requirements),
        RoleTaskContract::Reviewer {
            verification_requirements,
            ..
        } => ("reviewer", verification_requirements),
    };
    let mut schema: serde_json::Value =
        decode_current_json(codex_output_schema().as_bytes(), "built-in Codex schema")
            .map_err(|_| NativeHarnessError::invalid_input("invalid built-in Codex schema"))?;
    let result = schema["$defs"]["result"]["anyOf"]
        .as_array()
        .and_then(|choices| {
            choices
                .iter()
                .find(|choice| choice["properties"]["role"]["enum"][0] == role)
        })
        .cloned()
        .ok_or_else(|| NativeHarnessError::invalid_input("missing Codex role result schema"))?;
    schema["$defs"]["result"] = result;
    // Process cancellation/deadlines are observed by the supervisor, not
    // self-reported by a completed model turn. Core permits missing-context
    // requests only from Reviewer.
    schema["properties"]["outcome"]["anyOf"]
        .as_array_mut()
        .ok_or_else(|| NativeHarnessError::invalid_input("missing Codex outcome schema"))?
        .retain(|choice| {
            let status = choice["properties"]["status"]["enum"][0].as_str();
            !matches!(status, Some("cancelled" | "timed-out"))
                && (role == "reviewer" || status != Some("missing-context"))
        });
    if role == "reviewer"
        && requirements.iter().any(|requirement| {
            requirement.unit == context_core::harness::VerificationUnit::ProfessionalProfileArtifact
        })
    {
        schema["$defs"]["result"]["properties"]["improvements"]["description"] = serde_json::json!(
            "Core requires a non-empty improvements list to accompany a failed professional-profile-artifact requirement result. Report actual unresolved improvements with that result passed=false and supporting subject evidence; if there are no unresolved improvements, use an empty list. Never mark every requirement passed while reporting improvements, and never suppress a real improvement to obtain acceptance."
        );
    }
    if role != "writer" {
        let results = &mut schema["$defs"]["result"]["properties"]["requirement_results"];
        results["minItems"] = requirements.len().into();
        results["maxItems"] = requirements.len().into();
        if !requirements.is_empty() {
            let mut item = schema["$defs"]["requirement"].clone();
            item["properties"]["unit"] = serde_json::json!({
                "type": "string",
                "enum": requirements.iter().map(|r| r.unit).collect::<Vec<_>>()
            });
            schema["$defs"]["result"]["properties"]["requirement_results"]["items"] = item;
        }
    }
    serde_json::to_string(&schema)
        .map_err(|_| NativeHarnessError::invalid_input("cannot encode Codex role schema"))
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(tag = "type")]
pub(crate) enum CodexEvent {
    #[serde(rename = "thread.started")]
    ThreadStarted { thread_id: String },
    #[serde(rename = "turn.started")]
    TurnStarted,
    #[serde(rename = "item.started")]
    ItemStarted { item: CodexItem },
    #[serde(rename = "item.completed")]
    ItemCompleted { item: CodexItem },
    #[serde(rename = "turn.completed")]
    TurnCompleted { usage: CodexUsage },
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum CodexItem {
    AgentMessage {
        id: String,
        text: String,
    },
    CommandExecution {
        id: String,
        command: String,
        aggregated_output: String,
        exit_code: Option<i32>,
        status: CodexCommandStatus,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CodexCommandStatus {
    InProgress,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub(crate) struct CodexUsage {
    #[serde(rename = "input_tokens")]
    pub(crate) input: u64,
    #[serde(rename = "cached_input_tokens")]
    pub(crate) cached_input: u64,
    #[serde(rename = "cache_write_input_tokens")]
    pub(crate) cache_write_input: u64,
    #[serde(rename = "output_tokens")]
    pub(crate) output: u64,
    #[serde(rename = "reasoning_output_tokens")]
    pub(crate) reasoning_output: u64,
}

#[cfg(unix)]
pub(crate) struct CodexDirectory(PathBuf);

#[cfg(unix)]
impl CodexDirectory {
    fn create() -> Result<Self> {
        use std::os::unix::fs::DirBuilderExt as _;
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "ontology-role-{}-{nonce}-{}",
            process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|error| NativeHarnessError::io("create private Codex cwd", &path, error))?;
        let mut directory = Self(path);
        directory.0 = directory.0.canonicalize().map_err(|error| {
            NativeHarnessError::io("resolve private Codex cwd", &directory.0, error)
        })?;
        Ok(directory)
    }
}

#[cfg(unix)]
impl Drop for CodexDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Default)]
pub(crate) struct CodexNativeOutput {
    pub(crate) context: Option<(String, u64)>,
    pub(crate) turn_started: bool,
    pub(crate) first_output: Option<u64>,
    pub(crate) message: Option<String>,
    pub(crate) terminal: Option<(u64, bool)>,
    pub(crate) usage: Option<CodexUsage>,
    pub(crate) progress: CodexProgress,
}

// Only allowlisted event metadata is retained. In particular, commands, tool
// output, message text, IDs and error payloads never enter diagnostics.
#[derive(Clone, Default, serde::Serialize)]
pub(crate) struct CodexProgress {
    last_event: Option<&'static str>,
    last_event_at_millis: Option<u64>,
    last_item_kind: Option<&'static str>,
    last_tool_status: Option<CodexCommandStatus>,
    last_tool_exit_code: Option<i32>,
}

impl CodexProgress {
    fn observe(&mut self, event: &CodexEvent, now: u64) {
        self.last_event_at_millis = Some(now);
        self.last_event = Some(match event {
            CodexEvent::ThreadStarted { .. } => "thread.started",
            CodexEvent::TurnStarted => "turn.started",
            CodexEvent::ItemStarted { .. } => "item.started",
            CodexEvent::ItemCompleted { .. } => "item.completed",
            CodexEvent::TurnCompleted { .. } => "turn.completed",
        });
        if let CodexEvent::ItemStarted { item } | CodexEvent::ItemCompleted { item } = event {
            self.last_item_kind = Some(match item {
                CodexItem::AgentMessage { .. } => "agent_message",
                CodexItem::CommandExecution {
                    status, exit_code, ..
                } => {
                    self.last_tool_status = Some(status.clone());
                    self.last_tool_exit_code = *exit_code;
                    "command_execution"
                }
            });
        }
    }
}

impl CodexNativeOutput {
    pub(crate) fn observe(
        &mut self,
        line: &[u8],
        now: u64,
    ) -> std::result::Result<(), &'static str> {
        let event: CodexEvent = decode_current_json(line, "native Codex event")
            .map_err(|_| "invalid or unsupported native Codex JSONL")?;
        let mut progress = self.progress.clone();
        progress.observe(&event, now);
        match event {
            CodexEvent::ThreadStarted { thread_id } => {
                if self.context.is_some() || self.turn_started || self.terminal.is_some() {
                    return Err("duplicate or out-of-order native thread.started");
                }
                if thread_id.trim().is_empty() {
                    return Err("native thread.started has no context ID");
                }
                self.context = Some((thread_id, now));
            }
            CodexEvent::TurnStarted => {
                if self.context.is_none() || self.turn_started || self.terminal.is_some() {
                    return Err("out-of-order native turn.started");
                }
                self.turn_started = true;
            }
            CodexEvent::ItemStarted { .. } => {
                if !self.turn_started || self.terminal.is_some() {
                    return Err("out-of-order native item.started");
                }
            }
            CodexEvent::ItemCompleted { item } => {
                if !self.turn_started || self.terminal.is_some() {
                    return Err("out-of-order native item.completed");
                }
                if let CodexItem::AgentMessage { text, .. } = item {
                    if text.trim().is_empty() {
                        return Err("native agent_message has no text");
                    }
                    self.first_output.get_or_insert(now);
                    self.message = Some(text);
                }
            }
            CodexEvent::TurnCompleted { usage } => {
                if !self.turn_started || self.terminal.is_some() {
                    return Err("missing or duplicate native turn boundary");
                }
                self.terminal = Some((now, true));
                self.usage = Some(usage);
            }
        }
        self.progress = progress;
        Ok(())
    }

    pub(crate) fn outcome(
        &self,
        success: bool,
    ) -> std::result::Result<RoleExecutionOutcome, &'static str> {
        if !success || !matches!(self.terminal, Some((_, true))) {
            return Err("native Codex turn or process failed");
        }
        let Some(text) = &self.message else {
            return Err("native Codex completed without an agent_message");
        };
        let response = decode_current_json::<CodexResponse>(text.as_bytes(), "native Codex role response")
            .map_err(|_| {
                // Diagnose shape only. Never include model text, keys, values, or
                // serde error strings in durable execution failures.
                if decode_current_json::<serde_json::Value>(text.as_bytes(), "native Codex role response JSON").is_err() {
                    "native Codex role response contains invalid JSON syntax or duplicate keys"
                } else {
                    "native Codex role response is incompatible with Core output types or strict JSON validation"
                }
            })?;
        match response.outcome {
            RoleExecutionOutcome::Cancelled | RoleExecutionOutcome::TimedOut => {
                Err("native Codex model response cannot report runtime-owned termination")
            }
            outcome => Ok(outcome),
        }
    }
}

// Diagnostic observations are independent of Core acceptance. No source bytes,
// prompts, model output or native context identifiers enter this record.
#[derive(serde::Serialize)]
struct CodexObservation {
    kind: &'static str,
    role: HarnessRole,
    elapsed_millis: u64,
    serialized_input_bytes: usize,
    delivered_input_bytes: usize,
    reported_terminal_state: Option<RoleTerminalState>,
    usage: Option<CodexUsage>,
    progress: CodexProgress,
    child_pid: Option<u32>,
    process_group: Option<i32>,
    process_exit_code: Option<i32>,
    process_exit_signal: Option<i32>,
    launcher_identity: Option<ExecutableIdentity>,
    adapter_identity: Option<ExecutableIdentity>,
    // Neither approval decisions nor model identity are exposed by the
    // supported event schema. Absence must not be reported as approval or a
    // particular model. The adapter does not override inherited CLI settings.
    approval_state: &'static str,
    model_selection: &'static str,
}

#[derive(Clone, serde::Serialize)]
struct ExecutableIdentity {
    path: PathBuf,
    size_bytes: u64,
    modified_at_millis: Option<u128>,
}
impl ExecutableIdentity {
    fn adapter() -> Option<Self> {
        env::current_exe()
            .ok()
            .and_then(|path| Self::inspect(&path))
    }

    fn inspect(path: &Path) -> Option<Self> {
        let path = path.canonicalize().ok()?;
        let metadata = fs::metadata(&path).ok()?;
        // Observe metadata only: never add a file scan to lifecycle startup.
        if !metadata.is_file() {
            return None;
        }
        Some(Self {
            path,
            size_bytes: metadata.len(),
            modified_at_millis: metadata
                .modified()
                .ok()
                .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|elapsed| elapsed.as_millis()),
        })
    }
}
impl CodexObservation {
    fn write(&self, writer: &mut impl Write) {
        if let Ok(mut bytes) = serde_json::to_vec(self) {
            bytes.push(b'\n');
            // Broken diagnostics cannot reject an otherwise valid role result.
            let _ = writer.write_all(&bytes);
        }
    }
}
// Workers only record into this bounded native collector. The command emits
// after durable processing, sibling joins and native session release, so sink backpressure cannot stall
// a role lifecycle, a submission, cancellation, or the next frontier.
#[derive(Default)]
pub(crate) struct RoleObservations(std::sync::Mutex<Vec<CodexObservation>>);
impl RoleObservations {
    fn record(&self, observation: CodexObservation) {
        if let Ok(mut observations) = self.0.lock()
            && observations.len() < 4
        {
            // Core has four role kinds; the adapter rejects repeat executions.
            observations.push(observation);
        }
    }
    pub(crate) fn emit(&self) {
        let observations = self.0.lock().map(|mut values| std::mem::take(&mut *values));
        if let Ok(observations) = observations {
            let mut stderr = std::io::stderr().lock();
            for observation in observations {
                observation.write(&mut stderr);
            }
        }
    }
}
#[cfg(unix)]
struct CodexObservations<'a> {
    sink: Option<&'a RoleObservations>,
    native: CodexNativeOutput,
    started: std::time::Instant,
    role: HarnessRole,
    input_bytes: usize,
    delivered_bytes: usize,
    terminal_state: Option<RoleTerminalState>,
    child_pid: Option<u32>,
    process_group: Option<i32>,
    process_exit: std::rc::Rc<std::cell::Cell<Option<process::ExitStatus>>>,
    launcher_identity: Option<ExecutableIdentity>,
    adapter_identity: Option<ExecutableIdentity>,
}
#[cfg(unix)]
impl Drop for CodexObservations<'_> {
    fn drop(&mut self) {
        use std::os::unix::process::ExitStatusExt as _;
        if let Some(sink) = self.sink {
            sink.record(CodexObservation {
                kind: "native-role-observation",
                role: self.role,
                elapsed_millis: u64::try_from(self.started.elapsed().as_millis())
                    .unwrap_or(u64::MAX),
                serialized_input_bytes: self.input_bytes,
                delivered_input_bytes: self.delivered_bytes,
                reported_terminal_state: self.terminal_state,
                usage: self.native.usage.take(),
                progress: std::mem::take(&mut self.native.progress),
                child_pid: self.child_pid,
                process_group: self.process_group,
                process_exit_code: self.process_exit.get().and_then(|exit| exit.code()),
                process_exit_signal: self.process_exit.get().and_then(|exit| exit.signal()),
                launcher_identity: self.launcher_identity.take(),
                adapter_identity: self.adapter_identity.take(),
                approval_state: "not-exposed-by-supported-events",
                model_selection: "inherited-cli-configuration-unobserved",
            });
        }
    }
}

pub(crate) fn codex_terminal_state(outcome: &RoleExecutionOutcome) -> RoleTerminalState {
    match outcome {
        RoleExecutionOutcome::Completed { .. } => RoleTerminalState::Completed,
        RoleExecutionOutcome::MissingContext { .. } => RoleTerminalState::MissingContext,
        RoleExecutionOutcome::Failed { .. } => RoleTerminalState::Failed,
        RoleExecutionOutcome::Cancelled => RoleTerminalState::Cancelled,
        RoleExecutionOutcome::TimedOut => RoleTerminalState::TimedOut,
        RoleExecutionOutcome::Unsupported { .. } => RoleTerminalState::Unsupported,
    }
}

pub(crate) fn execute_codex_role(
    binary: &Path,
    prepared: &PreparedHarnessRun,
    invocation: &RoleInvocationContract,
    artifacts: &[FrozenReviewArtifact],
    cancelled: &AtomicBool,
    observations: &RoleObservations,
) -> Result<RoleExecutionResult> {
    validate_codex_producer_frontier(prepared, [invocation.role])?;
    let expected_artifacts = invocation
        .segments
        .iter()
        .filter_map(|segment| match segment {
            context_core::harness::RoleInvocationSegment::BoundReviewArtifact { artifact } => {
                Some(artifact)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if expected_artifacts
        != artifacts
            .iter()
            .map(FrozenReviewArtifact::binding)
            .collect::<Vec<_>>()
    {
        return Err(NativeHarnessError::invalid_input(
            "frozen review artifacts do not match the invocation",
        ));
    }
    // Serialize the exact typed segments once: no wrapper, prompt preamble, target
    // reread, summary, content replacement or argument-length-dependent transport.
    let input = serde_json::to_vec(&invocation.segments)
        .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?;
    if input.len()
        > prepared
            .role_run
            .runtime_capabilities
            .max_role_invocation_bytes
        || input.len() as u64 > MAX_HARNESS_JSON_BYTES
    {
        return Err(NativeHarnessError::invalid_input(
            "unsupported runtime: Codex stdin exceeds the finite input limit",
        ));
    }
    #[cfg(unix)]
    {
        let mut limits = prepared.plan.lifecycle.clone();
        limits.max_role_execution_millis = limits.max_role_execution_millis.min(
            prepared
                .role_run
                .runtime_capabilities
                .max_role_execution_millis,
        );
        limits.max_role_grace_millis = limits
            .max_role_grace_millis
            .min(prepared.role_run.runtime_capabilities.max_role_grace_millis);
        let task = &prepared
            .role_run
            .role_metadata
            .iter()
            .find(|metadata| metadata.role == invocation.role)
            .ok_or_else(|| NativeHarnessError::invalid_input("missing prepared Codex role task"))?
            .task;
        let schema = codex_role_output_schema(task)?;
        let (lifecycle, outcome) = run_codex_process_observed(
            binary,
            &input,
            artifacts,
            (invocation.role, &schema),
            &limits,
            cancelled,
            Some(observations),
        )?;
        invocation
            .bind_result(lifecycle, outcome)
            .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))
    }
    #[cfg(not(unix))]
    {
        let _ = (binary, input, artifacts, cancelled, observations);
        Err(NativeHarnessError::invalid_input(
            "unsupported runtime: Codex requires Unix process groups",
        ))
    }
}

#[cfg(unix)]
pub(crate) struct CodexProcess {
    child: process::Child,
    group: i32,
    cleaned: bool,
    exit_observation: std::rc::Rc<std::cell::Cell<Option<process::ExitStatus>>>,
}

#[cfg(unix)]
impl CodexProcess {
    fn signal(&self, signal: i32) -> std::io::Result<()> {
        // SAFETY: the child created its own group; the positive, checked child
        // PID is negated only to address that group, never the parent's group.
        if unsafe { libc::kill(-self.group, signal) } == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(error)
        }
    }
}

#[cfg(unix)]
impl Drop for CodexProcess {
    fn drop(&mut self) {
        // Also cover setup errors/unwinding. The normal path kills the group and
        // observes/reaps the leader within its close deadline below. Never use a
        // blocking wait here, including when the OS cannot reap before the bound.
        if !self.cleaned {
            let _ = self.signal(libc::SIGKILL);
            if let Ok(Some(exit)) = self.child.try_wait() {
                self.exit_observation.set(Some(exit));
            }
        }
    }
}

#[cfg(unix)]
pub(crate) fn codex_nonblocking(pipe: &impl std::os::fd::AsRawFd) -> std::io::Result<()> {
    let fd = pipe.as_raw_fd();
    // SAFETY: the borrowed pipe owns this live descriptor throughout both calls.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
pub(crate) fn codex_read_chunk(
    pipe: &mut impl Read,
    buffer: &mut [u8],
    total: &mut u64,
) -> std::io::Result<Option<usize>> {
    match pipe.read(buffer) {
        Ok(count) => {
            *total = total
                .checked_add(count as u64)
                .filter(|total| *total <= MAX_HARNESS_JSON_BYTES)
                .ok_or_else(|| {
                    std::io::Error::other("Codex output exceeds the finite stream limit")
                })?;
            Ok(Some(count))
        }
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
pub(crate) fn codex_failure_message(
    message: &str,
    exit: process::ExitStatus,
    stderr: &[u8],
) -> String {
    let mut detail = format!("{message} ({exit})");
    if !stderr.is_empty() {
        detail.push_str("; stderr tail: ");
        detail.push_str(String::from_utf8_lossy(stderr).trim());
    }
    detail
}

#[cfg(all(unix, test))]
pub(crate) fn run_codex_process(
    binary: &Path,
    input: &[u8],
    role: HarnessRole,
    limits: &RoleLifecycleLimits,
    cancelled: &AtomicBool,
) -> Result<(ReportedRoleLifecycle, RoleExecutionOutcome)> {
    run_codex_process_observed(
        binary,
        input,
        &[],
        (role, codex_output_schema()),
        limits,
        cancelled,
        None,
    )
}

#[cfg(unix)]
fn materialize_review_artifacts(
    directory: &Path,
    artifacts: &[FrozenReviewArtifact],
) -> Result<Vec<PathBuf>> {
    use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
    let mut images = Vec::new();
    for artifact in artifacts {
        // Paths and bytes come exclusively from Core's prepared-scope loader.
        let path = directory.join(&artifact.binding().workspace_relative_path);
        let parent = path
            .parent()
            .ok_or_else(|| NativeHarnessError::invalid_input("missing review artifact parent"))?;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)
            .map_err(|error| {
                NativeHarnessError::io("create private review artifact directory", parent, error)
            })?;
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .and_then(|mut file| file.write_all(artifact.bytes()))
            .map_err(|error| {
                NativeHarnessError::io("write frozen review artifact", &path, error)
            })?;
        if artifact.binding().format != ReviewArtifactFormat::Pdf {
            images.push(path);
        }
    }
    Ok(images)
}

#[cfg(unix)]
#[allow(
    clippy::too_many_lines,
    reason = "one bounded process supervisor owns stdin, native events, deadlines, group termination and reaping"
)]
fn run_codex_process_observed(
    binary: &Path,
    input: &[u8],
    artifacts: &[FrozenReviewArtifact],
    role_schema: (HarnessRole, &str),
    limits: &RoleLifecycleLimits,
    cancelled: &AtomicBool,
    sink: Option<&RoleObservations>,
) -> Result<(ReportedRoleLifecycle, RoleExecutionOutcome)> {
    let (role, schema) = role_schema;
    use std::os::unix::{
        fs::OpenOptionsExt as _,
        process::{CommandExt as _, ExitStatusExt as _},
    };
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    let directory = CodexDirectory::create()?;
    let image_paths = materialize_review_artifacts(&directory.0, artifacts)?;
    let schema_path = directory.0.join("outcome-schema.json");
    let schema = schema.as_bytes();
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&schema_path)
        .and_then(|mut file| file.write_all(schema))
        .map_err(|error| {
            NativeHarnessError::io("write Codex output schema", &schema_path, error)
        })?;
    let launcher_identity = ExecutableIdentity::inspect(binary);
    let adapter_identity = ExecutableIdentity::adapter();
    let started_at_millis = u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| NativeHarnessError::invalid_input(error.to_string()))?
            .as_millis(),
    )
    .map_err(|_| NativeHarnessError::invalid_input("Codex start time overflow"))?;
    let started = Instant::now();
    // Polling cadence is not an OS scheduling guarantee. Reserve up to 50ms
    // (10% of each budget, at least one poll) before execution/grace limits and half the
    // close budget for forced group termination/reaping. Report actual times;
    // an OS overrun still fails rather than being clamped into acceptance.
    let quantum = Duration::from_millis(2);
    let total = Duration::from_millis(limits.max_total_role_millis);
    let close = Duration::from_millis(limits.max_role_close_millis);
    let grace = Duration::from_millis(limits.max_role_grace_millis);
    let execution_budget =
        Duration::from_millis(limits.max_role_execution_millis).min(total.saturating_sub(close));
    let scheduling_margin =
        |budget: Duration| (budget / 10).min(Duration::from_millis(50)).max(quantum);
    let execution = execution_budget.saturating_sub(scheduling_margin(execution_budget));
    let grace_margin = scheduling_margin(grace);
    let total_deadline = started
        .checked_add(total)
        .ok_or_else(|| NativeHarnessError::invalid_input("Codex total deadline overflow"))?;
    let execution_deadline = started
        .checked_add(execution)
        .ok_or_else(|| NativeHarnessError::invalid_input("Codex execution deadline overflow"))?;
    if execution.is_zero()
        || close.is_zero()
        || started_at_millis
            .checked_add(limits.max_total_role_millis)
            .is_none()
    {
        return Err(NativeHarnessError::invalid_input(
            "unsupported runtime: insufficient finite Codex lifecycle budget",
        ));
    }
    let mut observations = CodexObservations {
        sink,
        native: CodexNativeOutput::default(),
        started,
        role,
        input_bytes: input.len(),
        delivered_bytes: 0,
        terminal_state: None,
        child_pid: None,
        process_group: None,
        process_exit: Default::default(),
        launcher_identity,
        adapter_identity,
    };
    let mut command = process::Command::new(binary);
    command
        .args([
            "exec",
            "--ephemeral",
            "--skip-git-repo-check",
            "--json",
            "--color",
            "never",
            "--sandbox",
            "read-only",
        ])
        .arg("--output-schema")
        .arg(&schema_path);
    for path in image_paths {
        command.arg("--image").arg(path);
    }
    let child = command
        .arg("-")
        .current_dir(&directory.0)
        .process_group(0)
        .stdin(process::Stdio::piped())
        .stdout(process::Stdio::piped())
        .stderr(process::Stdio::piped())
        .spawn()
        .map_err(|error| NativeHarnessError::io("start Codex role", binary, error))?;
    let group =
        i32::try_from(child.id()).expect("a Unix child PID must fit the native signed pid_t");
    observations.child_pid = Some(child.id());
    observations.process_group = Some(group);
    let mut process = CodexProcess {
        child,
        group,
        cleaned: false,
        exit_observation: observations.process_exit.clone(),
    };
    let mut stdin = process.child.stdin.take();
    let mut stdout = process.child.stdout.take();
    let mut stderr = process.child.stderr.take();
    let setup = stdin
        .as_ref()
        .ok_or_else(|| std::io::Error::other("missing Codex stdin"))
        .and_then(codex_nonblocking)
        .and_then(|()| {
            stdout
                .as_ref()
                .ok_or_else(|| std::io::Error::other("missing Codex stdout"))
        })
        .and_then(codex_nonblocking)
        .and_then(|()| {
            stderr
                .as_ref()
                .ok_or_else(|| std::io::Error::other("missing Codex stderr"))
        })
        .and_then(codex_nonblocking);
    let mut failure = setup.err().map(|_| "cannot initialize bounded Codex pipes");
    if failure.is_some() {
        stdin = None;
        stdout = None;
        stderr = None;
    }
    let mut pending = Vec::new();
    let mut written = 0_usize;
    let mut stdout_bytes = 0;
    let mut stderr_bytes = 0;
    let mut stderr_tail = Vec::new();
    let mut status = None;
    let mut interrupt = None;
    let mut grace_deadline = None;
    let mut terminal = None;
    let mut close_deadline = None;
    let mut kill_deadline = None;
    let mut killed = false;
    let mut group_termination_error = None;
    let mut timed_out = false;
    let mut sibling_cancelled = false;
    let now_millis = || {
        started_at_millis
            .saturating_add(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX))
    };
    loop {
        let now = Instant::now();
        if now >= total_deadline {
            return Err(NativeHarnessError::invalid_input(
                "Codex exceeded the total lifecycle limit; group termination requested",
            ));
        }
        let mut progressed = false;
        if failure.is_none()
            && interrupt.is_none()
            && terminal.is_none()
            && now < execution_deadline
            && let Some(pipe) = stdin.as_mut()
        {
            let end = input.len().min(written.saturating_add(8192));
            match pipe.write(&input[written..end]) {
                Ok(0) => {
                    failure = Some("Codex closed stdin before receiving all segments");
                }
                Ok(count) => {
                    written += count;
                    observations.delivered_bytes = written;
                    progressed = true;
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(_) => {
                    failure = Some("cannot deliver complete Codex stdin");
                }
            }
            if written == input.len() {
                stdin = None;
            }
        }
        // One bounded chunk per stream per iteration prevents an output flood
        // from starving input, process observation or any lifecycle deadline.
        let mut buffer = [0_u8; 8192];
        if let Some(pipe) = stdout.as_mut() {
            match codex_read_chunk(pipe, &mut buffer, &mut stdout_bytes) {
                Ok(Some(count)) => {
                    progressed |= count != 0;
                    let scanned = pending.len();
                    pending.extend_from_slice(&buffer[..count]);
                    let mut consumed = 0;
                    for (index, byte) in pending.iter().enumerate().skip(scanned) {
                        if *byte == b'\n' {
                            if failure.is_none() {
                                failure = observations
                                    .native
                                    .observe(&pending[consumed..index], now_millis())
                                    .err();
                            }
                            consumed = index + 1;
                        }
                    }
                    pending.drain(..consumed);
                    if count == 0 {
                        if !pending.is_empty() && failure.is_none() {
                            failure = observations.native.observe(&pending, now_millis()).err();
                        }
                        pending.clear();
                        stdout = None;
                    }
                }
                Ok(None) => {}
                Err(_) => {
                    failure = Some("Codex stdout failed or exceeded its finite limit");
                    stdout = None;
                }
            }
        }
        if let Some(pipe) = stderr.as_mut() {
            match codex_read_chunk(pipe, &mut buffer, &mut stderr_bytes) {
                Ok(Some(0)) => {
                    stderr = None;
                }
                Ok(Some(count)) => {
                    progressed = true;
                    stderr_tail.extend_from_slice(&buffer[..count]);
                    stderr_tail.drain(..stderr_tail.len().saturating_sub(4096));
                }
                Ok(None) => {}
                Err(_) => {
                    failure = Some("Codex stderr failed or exceeded its finite limit");
                    stderr = None;
                }
            }
        }
        if status.is_none() {
            match process.child.try_wait() {
                Ok(Some(exit)) => {
                    observations.process_exit.set(Some(exit));
                    status = Some(exit);
                    progressed = true;
                }
                Ok(None) => {}
                Err(_) => {
                    failure = Some("cannot observe Codex process exit");
                }
            }
        }
        let now = Instant::now();
        if terminal.is_none() && interrupt.is_none() {
            if let Some((at, _)) = observations.native.terminal {
                terminal = Some(at);
            } else if failure.is_some() || (status.is_some() && stdout.is_none()) {
                terminal = Some(now_millis());
            } else if now >= execution_deadline || cancelled.load(Ordering::Relaxed) {
                sibling_cancelled = cancelled.load(Ordering::Relaxed);
                timed_out = !sibling_cancelled;
                interrupt = Some(now_millis());
                let deadline = now
                    .checked_add(grace)
                    .unwrap_or(total_deadline)
                    .min(total_deadline.checked_sub(close).unwrap_or(now));
                grace_deadline = Some(deadline);
                stdin = None;
                if process.signal(libc::SIGINT).is_err() {
                    failure = Some("cannot interrupt Codex process group");
                }
            }
        }
        if let Some(deadline) = grace_deadline
            && terminal.is_none()
            && (status.is_some() || now >= deadline.checked_sub(grace_margin).unwrap_or(deadline))
        {
            terminal = Some(now_millis());
        }
        if (terminal.is_some() || status.is_some()) && close_deadline.is_none() {
            stdin = None;
            let terminal_instant = terminal
                .and_then(|at| {
                    started.checked_add(Duration::from_millis(at.saturating_sub(started_at_millis)))
                })
                .unwrap_or(now);
            close_deadline = Some(
                terminal_instant
                    .checked_add(close)
                    .unwrap_or(total_deadline)
                    .min(total_deadline),
            );
            kill_deadline = Some(
                now.checked_add(close / 2)
                    .unwrap_or(total_deadline)
                    .min(close_deadline.expect("close deadline was just assigned")),
            );
        }
        if (!killed
            || (group_termination_error
                .as_ref()
                .is_some_and(|error: &std::io::Error| error.raw_os_error() == Some(libc::EPERM))
                && close_deadline.is_some_and(|deadline| now < deadline)))
            && (status.is_some()
                || failure.is_some()
                || ((timed_out || sibling_cancelled) && terminal.is_some())
                || kill_deadline.is_some_and(|deadline| now >= deadline))
        {
            if status.is_none() && failure.is_none() && !timed_out && !sibling_cancelled {
                timed_out = true;
            }
            group_termination_error = process.signal(libc::SIGKILL).err();
            killed = true;
        }
        if let Some(exit) = status
            && stdout.is_none()
            && stderr.is_none()
        {
            // A zombie-only Darwin process group can transiently return EPERM.
            // Require a later successful signal or ESRCH within the same close
            // deadline; EPERM itself never establishes successful termination.
            if group_termination_error
                .as_ref()
                .is_some_and(|error| error.raw_os_error() == Some(libc::EPERM))
                && let Some(deadline) = close_deadline
                && Instant::now() < deadline
            {
                std::thread::sleep(quantum.min(deadline.saturating_duration_since(Instant::now())));
                continue;
            }
            let closed = now_millis();
            if !killed || group_termination_error.is_some() {
                let detail = group_termination_error.map_or_else(
                    || "Codex process group termination was not requested".to_owned(),
                    |error| format!("cannot confirm Codex process group termination: {error}"),
                );
                return Err(NativeHarnessError::invalid_input(codex_failure_message(
                    &detail,
                    exit,
                    &stderr_tail,
                )));
            }
            process.cleaned = true;
            let Some((context_id, context_ready_at_millis)) = observations.native.context.clone()
            else {
                return Err(NativeHarnessError::invalid_input(codex_failure_message(
                    "Codex execution failed without an observed native thread.started context ID",
                    exit,
                    &stderr_tail,
                )));
            };
            let outcome = if timed_out {
                RoleExecutionOutcome::TimedOut
            } else if sibling_cancelled {
                RoleExecutionOutcome::Cancelled
            } else if let Some(message) = failure {
                RoleExecutionOutcome::Failed {
                    message: codex_failure_message(message, exit, &stderr_tail),
                }
            } else if written != input.len() {
                RoleExecutionOutcome::Failed {
                    message: codex_failure_message(
                        "Codex did not receive the complete segments array",
                        exit,
                        &stderr_tail,
                    ),
                }
            } else if matches!(exit.signal(), Some(libc::SIGINT | libc::SIGTERM)) {
                RoleExecutionOutcome::Cancelled
            } else {
                observations
                    .native
                    .outcome(exit.success())
                    .unwrap_or_else(|message| RoleExecutionOutcome::Failed {
                        message: codex_failure_message(message, exit, &stderr_tail),
                    })
            };
            let terminal_at_millis = terminal.unwrap_or(closed);
            let grace_deadline_at_millis = grace_deadline.map(|deadline| {
                started_at_millis.saturating_add(
                    u64::try_from(deadline.duration_since(started).as_millis()).unwrap_or(u64::MAX),
                )
            });
            let lifecycle = ReportedRoleLifecycle {
                role,
                context_id,
                started_at_millis,
                context_ready_at_millis,
                first_output_at_millis: observations.native.first_output,
                interrupt_requested_at_millis: interrupt,
                grace_deadline_at_millis,
                terminal_at_millis,
                closed_at_millis: closed,
                terminal_state: codex_terminal_state(&outcome),
            };
            if closed.saturating_sub(started_at_millis) > limits.max_total_role_millis
                || closed.saturating_sub(terminal_at_millis) > limits.max_role_close_millis
                || interrupt
                    .unwrap_or(terminal_at_millis)
                    .saturating_sub(started_at_millis)
                    > limits.max_role_execution_millis
                || grace_deadline_at_millis.is_some_and(|deadline| terminal_at_millis > deadline)
            {
                return Err(NativeHarnessError::invalid_input(format!(
                    "observed Codex lifecycle exceeded prepared limits: execution={}ms/{}ms, grace_overrun={}ms, close={}ms/{}ms, total={}ms/{}ms",
                    interrupt
                        .unwrap_or(terminal_at_millis)
                        .saturating_sub(started_at_millis),
                    limits.max_role_execution_millis,
                    grace_deadline_at_millis
                        .map_or(0, |deadline| terminal_at_millis.saturating_sub(deadline)),
                    closed.saturating_sub(terminal_at_millis),
                    limits.max_role_close_millis,
                    closed.saturating_sub(started_at_millis),
                    limits.max_total_role_millis,
                )));
            }
            observations.terminal_state = Some(codex_terminal_state(&outcome));
            return Ok((lifecycle, outcome));
        }
        if close_deadline.is_some_and(|deadline| now >= deadline) {
            return Err(NativeHarnessError::invalid_input(
                "Codex group did not close and reap within the prepared close limit",
            ));
        }
        if !progressed {
            let deadline = close_deadline
                .or(grace_deadline)
                .unwrap_or(execution_deadline)
                .min(total_deadline);
            std::thread::sleep(quantum.min(deadline.saturating_duration_since(Instant::now())));
        }
    }
}

#[cfg(test)]
mod observation_tests {
    use super::*;
    #[test]
    fn rejected_events_cannot_replace_last_accepted_progress() {
        let mut native = CodexNativeOutput::default();
        let item = serde_json::json!({"type":"item.started","item":{"type":"command_execution","id":"private-item","command":"private-command","aggregated_output":"private-output","exit_code":null,"status":"in_progress"}});
        assert!(native.observe(item.to_string().as_bytes(), 1).is_err());
        assert!(native.progress.last_event.is_none());
        native
            .observe(
                br#"{"type":"thread.started","thread_id":"private-thread"}"#,
                2,
            )
            .unwrap();
        assert!(native.observe(item.to_string().as_bytes(), 3).is_err());
        assert_eq!(native.progress.last_event, Some("thread.started"));
        assert_eq!(native.progress.last_event_at_millis, Some(2));
        assert!(native.progress.last_tool_status.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn error_cleanup_preserves_observed_exit_status() {
        use std::os::unix::process::CommandExt as _;
        let child = process::Command::new("/bin/sh")
            .args(["-c", "exit 7"])
            .process_group(0)
            .spawn()
            .unwrap();
        let exit_observation = std::rc::Rc::new(std::cell::Cell::new(None));
        let mut process = CodexProcess {
            group: i32::try_from(child.id()).unwrap(),
            child,
            cleaned: false,
            exit_observation: exit_observation.clone(),
        };
        // Deterministically make the later cleanup's try_wait observe an exit.
        process.child.wait().unwrap();
        drop(process);
        assert_eq!(exit_observation.get().and_then(|exit| exit.code()), Some(7));
    }

    #[cfg(unix)]
    #[test]
    fn observed_cancellation_does_not_depend_on_a_model_terminal_response() {
        use std::os::unix::fs::PermissionsExt as _;
        let directory = CodexDirectory::create().unwrap();
        let binary = directory.0.join("fake-codex");
        let ready = directory.0.join("ready");
        fs::write(&binary, format!(
            "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{{\"type\":\"thread.started\",\"thread_id\":\"observed-cancel\"}}' '{{\"type\":\"turn.started\"}}'\ntouch '{}'\nexec sleep 30\n",
            ready.display()
        )).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let cancelled = AtomicBool::new(false);
        let limits = RoleLifecycleLimits {
            max_role_execution_millis: 3000,
            max_role_grace_millis: 100,
            max_role_close_millis: 500,
            max_total_role_millis: 3600,
        };
        let (_, outcome) = std::thread::scope(|scope| {
            scope.spawn(|| {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                while !ready.exists() && std::time::Instant::now() < deadline {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                assert!(
                    ready.exists(),
                    "synthetic role must have emitted its native context"
                );
                cancelled.store(true, Ordering::Relaxed);
            });
            run_codex_process(&binary, b"[]", HarnessRole::Verifier, &limits, &cancelled).unwrap()
        });
        assert_eq!(outcome, RoleExecutionOutcome::Cancelled);
    }

    #[test]
    fn progress_retains_allowlisted_metadata_without_command_or_response() {
        let mut native = CodexNativeOutput::default();
        let events = [
            serde_json::json!({"type":"thread.started","thread_id":"private-thread"}),
            serde_json::json!({"type":"turn.started"}),
            serde_json::json!({"type":"item.started","item":{"type":"command_execution","id":"private-item","command":"private-command","aggregated_output":"private-output","exit_code":null,"status":"in_progress"}}),
            serde_json::json!({"type":"item.completed","item":{"type":"command_execution","id":"private-item","command":"private-command","aggregated_output":"private-output","exit_code":17,"status":"failed"}}),
            serde_json::json!({"type":"item.completed","item":{"type":"agent_message","id":"private-message","text":"private-response"}}),
        ];
        for (at, event) in events.iter().enumerate() {
            native
                .observe(event.to_string().as_bytes(), at as u64)
                .unwrap();
        }
        let value = serde_json::to_value(&native.progress).unwrap();
        assert_eq!(value["last_event"], "item.completed");
        assert_eq!(value["last_event_at_millis"], 4);
        assert_eq!(value["last_item_kind"], "agent_message");
        assert_eq!(value["last_tool_status"], "failed");
        assert_eq!(value["last_tool_exit_code"], 17);
        assert!(!value.to_string().contains("private-"));
    }

    #[cfg(unix)]
    #[test]
    fn timeout_retains_running_tool_and_process_identity_without_payload() {
        use std::os::unix::fs::PermissionsExt as _;
        let directory = CodexDirectory::create().unwrap();
        let binary = directory.0.join("fake-codex");
        let events = [
            serde_json::json!({"type":"thread.started","thread_id":"private-thread"}),
            serde_json::json!({"type":"turn.started"}),
            serde_json::json!({"type":"item.started","item":{"type":"command_execution","id":"private-item","command":"private-command","aggregated_output":"private-output","exit_code":null,"status":"in_progress"}}),
        ].map(|value| value.to_string()).join("\n");
        fs::write(
            &binary,
            format!("#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{events}'\nexec sleep 10\n"),
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let sink = RoleObservations::default();
        // Allow the synthetic launcher to start under parallel-suite load. The
        // ten-second sleep still exceeds this budget and must be terminated.
        let limits = RoleLifecycleLimits {
            max_role_execution_millis: 5000,
            max_role_grace_millis: 100,
            max_role_close_millis: 500,
            max_total_role_millis: 5600,
        };
        let (_, outcome) = run_codex_process_observed(
            &binary,
            b"[]",
            &[],
            (HarnessRole::Writer, codex_output_schema()),
            &limits,
            &AtomicBool::new(false),
            Some(&sink),
        )
        .unwrap();
        assert_eq!(outcome, RoleExecutionOutcome::TimedOut);
        let values = sink.0.lock().unwrap();
        assert_eq!(values.len(), 1);
        let value = serde_json::to_value(&values[0]).unwrap();
        assert_eq!(value["progress"]["last_event"], "item.started");
        assert_eq!(value["progress"]["last_tool_status"], "in_progress");
        assert!(value["child_pid"].as_u64().is_some());
        assert_eq!(value["child_pid"], value["process_group"]);
        assert!(value["process_exit_signal"].as_i64().is_some());
        assert_eq!(
            value["launcher_identity"]["size_bytes"],
            fs::metadata(&binary).unwrap().len()
        );
        assert!(!value.to_string().contains("private-"));
        assert!(value["usage"].is_null());
    }

    #[test]
    fn observation_is_content_free_and_unknown_usage_stays_null() {
        let observation = CodexObservation {
            kind: "native-role-observation",
            role: HarnessRole::Writer,
            elapsed_millis: 12,
            serialized_input_bytes: 100,
            delivered_input_bytes: 40,
            reported_terminal_state: None,
            usage: None,
            progress: CodexProgress::default(),
            child_pid: None,
            process_group: None,
            process_exit_code: None,
            process_exit_signal: None,
            launcher_identity: None,
            adapter_identity: None,
            approval_state: "not-exposed-by-supported-events",
            model_selection: "inherited-cli-configuration-unobserved",
        };
        let mut bytes = Vec::new();
        observation.write(&mut bytes);
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(value["usage"].is_null());
        assert_eq!(value["delivered_input_bytes"], 40);
        assert!(value["process_exit_code"].is_null());
        assert_eq!(value["approval_state"], "not-exposed-by-supported-events");
        assert!(bytes.len() < 2048);
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        observation.write(&mut Broken);
    }
    #[cfg(unix)]
    #[test]
    fn backpressured_stderr_cannot_delay_native_role_return() {
        use std::os::unix::fs::PermissionsExt as _;
        const CHILD: &str = "ONTOLOGY_TEST_ROLE_OBSERVATION_BACKPRESSURE";
        if env::var_os(CHILD).is_some() {
            // Only this dedicated test child owns the modified descriptor flags.
            let flags = unsafe { libc::fcntl(libc::STDERR_FILENO, libc::F_GETFL) };
            assert!(flags >= 0);
            assert_eq!(
                unsafe {
                    libc::fcntl(libc::STDERR_FILENO, libc::F_SETFL, flags | libc::O_NONBLOCK)
                },
                0
            );
            let fill = [b'x'; 4096];
            while unsafe { libc::write(libc::STDERR_FILENO, fill.as_ptr().cast(), fill.len()) } >= 0
            {
            }
            assert_eq!(
                std::io::Error::last_os_error().kind(),
                std::io::ErrorKind::WouldBlock
            );
            assert_eq!(
                unsafe { libc::fcntl(libc::STDERR_FILENO, libc::F_SETFL, flags) },
                0
            );
            let directory = CodexDirectory::create().unwrap();
            let binary = directory.0.join("fake-codex");
            let message = serde_json::to_string(&CodexResponse {
                outcome: RoleExecutionOutcome::Unsupported {
                    reason: "synthetic unsupported response".to_owned(),
                },
            })
            .unwrap();
            let events = [
                serde_json::json!({"type":"thread.started","thread_id":"backpressure-test"}),
                serde_json::json!({"type":"turn.started"}),
                serde_json::json!({"type":"item.completed","item":{"id":"answer","type":"agent_message","text":message}}),
                serde_json::json!({"type":"turn.completed","usage":{"input_tokens":100,"cached_input_tokens":20,"cache_write_input_tokens":0,"output_tokens":10,"reasoning_output_tokens":5}}),
            ].map(|value| value.to_string()).join("\n");
            fs::write(
                &binary,
                format!("#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{events}'\n"),
            )
            .unwrap();
            fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
            let sink = RoleObservations::default();
            let limits = RoleLifecycleLimits {
                // Allow process startup under the parallel suite; the parent
                // still enforces a five-second bound on blocked diagnostics.
                max_role_execution_millis: 3000,
                max_role_grace_millis: 100,
                max_role_close_millis: 500,
                max_total_role_millis: 3600,
            };
            let result = run_codex_process_observed(
                &binary,
                b"[]",
                &[],
                (HarnessRole::Reviewer, codex_output_schema()),
                &limits,
                &AtomicBool::new(false),
                Some(&sink),
            );
            // Keep stderr backpressured until the worker returns, then let test
            // failures reach the parent instead of blocking the panic report.
            assert_eq!(
                unsafe { libc::dup2(libc::STDOUT_FILENO, libc::STDERR_FILENO) },
                libc::STDERR_FILENO
            );
            let (_, outcome) = result.unwrap();
            assert_eq!(
                outcome,
                RoleExecutionOutcome::Unsupported {
                    reason: "synthetic unsupported response".to_owned(),
                }
            );
            let values = sink.0.lock().unwrap();
            assert_eq!(values.len(), 1);
            assert_eq!(values[0].delivered_input_bytes, 2);
            assert_eq!(values[0].usage.as_ref().unwrap().input, 100);
            println!("native worker returned with retained observations");
            return;
        }
        let mut child = process::Command::new(env::current_exe().unwrap())
            .args(["--exact", "native_role::observation_tests::backpressured_stderr_cannot_delay_native_role_return", "--nocapture", "--test-threads=1"])
            .env(CHILD, "1").stdout(process::Stdio::piped()).stderr(process::Stdio::piped())
            .spawn().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("native adapter waited for an unconsumed stderr pipe");
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let mut stdout = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut stdout)
            .unwrap();
        assert!(status.success(), "{stdout}");
        assert!(stdout.contains("native worker returned with retained observations"));
    }
}
