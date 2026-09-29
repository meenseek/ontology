use crate::native_harness::{
    MAX_HARNESS_JSON_BYTES, NativeHarnessError as ContextVaultError, NativeHarnessResult as Result,
};
use context_core::harness::{
    HarnessAction, HarnessEngine, HarnessExecutionEvent, HarnessExecutionRecord, HarnessRole,
    PreparedHarnessRun, ReportedRoleLifecycle, RequirementResult, ResultEvidenceReference,
    RoleExecutionOutcome, RoleExecutionResult, RoleInvocationContract, RoleLifecycleLimits,
    RoleTerminalState, VerificationOwner, decode_current_json,
};
use std::{
    env, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process,
};
// This adapter only transports Core-issued segments and observes a native process.
// It does not select policies, evaluate evidence, or authorize source mutations.
pub(crate) fn validate_codex_binary(binary: &Path) -> Result<()> {
    if !binary.is_absolute() {
        return Err(ContextVaultError::invalid_input(
            "--codex-binary requires an absolute executable path",
        ));
    }
    let metadata = fs::metadata(binary)
        .map_err(|error| ContextVaultError::io("inspect Codex executable", binary, error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 {
            return Ok(());
        }
    }
    let _ = metadata;
    Err(ContextVaultError::invalid_input(
        "unsupported runtime: Codex execution requires a Unix executable file",
    ))
}

pub(crate) fn execute_codex_frontier(
    engine: &HarnessEngine,
    prepared: &PreparedHarnessRun,
    mut record: HarnessExecutionRecord,
    binary: &Path,
) -> Result<HarnessExecutionRecord> {
    // Only callers holding the typed return of begin_durable/advance_durable enter
    // here. Never deserialize a caller-supplied frontier or replay a stale sibling.
    let mut executed = std::collections::BTreeSet::new();
    loop {
        if record.prepared_run_digest != prepared.prepared_run_digest {
            return Err(ContextVaultError::invalid_input(
                "Core frontier and Codex lifecycle preparation differ",
            ));
        }
        if record.ready_tool_invocation.is_some()
            || record
                .ready_role_invocations
                .iter()
                .any(|invocation| !codex_role_supported(prepared, invocation.role))
        {
            return Err(ContextVaultError::invalid_input(
                "unsupported runtime: --codex-binary supports only DocumentWrite Writer and Reviewer/Verifier frontiers without tools",
            ));
        }
        let Some(invocation) = record.ready_role_invocations.first() else {
            // A non-completed result also empties the frontier. Core owns the
            // terminal evaluation and may reject evaluation of an incomplete write.
            if prepared.accepted_tool_plan.is_some() {
                return Ok(record);
            }
            return HarnessExecutionRecord::evaluate_durable(engine, &record.run_identifier, None)
                .map_err(|error| ContextVaultError::invalid_input(error.to_string()));
        };
        if !executed.insert(invocation.role) {
            return Err(ContextVaultError::invalid_input(
                "unsupported runtime: automatic role retries are not supported",
            ));
        }
        let result = execute_codex_role(binary, prepared, invocation)?;
        record = HarnessExecutionRecord::advance_durable(
            engine,
            &record.run_identifier,
            HarnessExecutionEvent::RoleResult {
                result: Box::new(result),
            },
        )
        .map_err(|error| ContextVaultError::invalid_input(error.to_string()))?;
    }
}

fn codex_role_supported(prepared: &PreparedHarnessRun, role: HarnessRole) -> bool {
    matches!(role, HarnessRole::Reviewer | HarnessRole::Verifier)
        || (role == HarnessRole::Writer && prepared.plan.action == HarnessAction::DocumentWrite)
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
  "properties": {"outcome": {"anyOf": [{"type": "object","properties": {"status": {"type": "string","enum": ["completed"]},"result": {"$ref": "#/$defs/result"}},"required": ["status","result"],"additionalProperties": false},{"type": "object","properties": {"status": {"type": "string","enum": ["missing-context"]},"request": {"$ref": "#/$defs/request"}},"required": ["status","request"],"additionalProperties": false},{"type": "object","properties": {"status": {"type": "string","enum": ["failed"]},"message": {"type": "string"}},"required": ["status","message"],"additionalProperties": false},{"type": "object","properties": {"status": {"type": "string","enum": ["cancelled"]}},"required": ["status"],"additionalProperties": false},{"type": "object","properties": {"status": {"type": "string","enum": ["timed-out"]}},"required": ["status"],"additionalProperties": false},{"type": "object","properties": {"status": {"type": "string","enum": ["unsupported"]},"reason": {"type": "string"}},"required": ["status","reason"],"additionalProperties": false}]}},
  "required": ["outcome"],
  "additionalProperties": false,
  "$defs": {
    "verification-unit": {"type":"string","enum":["completion-contract","scope-compliance","terminology-and-readability","code-correctness","tests-and-static-analysis","source-ownership","fact-and-claim-boundary","evidence-and-uncertainty","idea-not-promoted-to-decision","solo-mvp-ideation-contract","retrieval-scope-and-digest","curation-provenance","source-and-ontology-boundary","career-evidence-lineage","career-surface-contract","career-perspective-routing","career-public-safety","career-output-surface-selection","resume-first-screen-and-artifact","career-description-case-structure","portfolio-local-build-and-public-copy","professional-profile-artifact","career-holistic-coherence","output-adapter-contract","output-adapter-surface-selection","resume-output-adapter-artifact","career-description-output-adapter-artifact","professional-profile-output-adapter-artifact"]},
    "evidence": {"anyOf": [{"type": "object","properties": {"source": {"type": "string","enum": ["target"]},"workspace_relative_path": {"type": "string"},"content_digest": {"type": "string"},"locator": {"type": "string"}},"required": ["source","workspace_relative_path","content_digest","locator"],"additionalProperties": false},{"type": "object","properties": {"source": {"type": "string","enum": ["bound-document"]},"relative_path": {"type": "string"},"content_digest": {"type": "string"},"locator": {"type": "string"}},"required": ["source","relative_path","content_digest","locator"],"additionalProperties": false},{"type": "object","properties": {"source": {"type": "string","enum": ["tool-result"]},"unit": {"$ref":"#/$defs/verification-unit"},"result_digest": {"type": "string"}},"required": ["source","unit","result_digest"],"additionalProperties": false},{"type": "object","properties": {"source": {"type": "string","enum": ["produced-artifact"]},"artifact_digest": {"type": "string"},"locator": {"type": "string"}},"required": ["source","artifact_digest","locator"],"additionalProperties": false}]},
    "requirement": {"type": "object","properties": {"unit": {"$ref":"#/$defs/verification-unit"},"passed": {"type": "boolean"},"detail": {"type": "string"},"evidence": {"type": "array","items": {"$ref": "#/$defs/evidence"}}},"required": ["unit","passed","detail","evidence"],"additionalProperties": false},
    "observation": {"type": "object","properties": {"message": {"type": "string"},"evidence": {"type": "array","items": {"$ref": "#/$defs/evidence"}}},"required": ["message","evidence"],"additionalProperties": false},
    "learning": {"type": "object","properties": {"title": {"type": "string"},"guidance": {"type": "string"},"failed_unit": {"$ref":"#/$defs/verification-unit"},"requirement_result_digest": {"type": "string"},"evidence": {"type": "array","items": {"$ref": "#/$defs/evidence"}}},"required": ["title","guidance","failed_unit","requirement_result_digest","evidence"],"additionalProperties": false},
    "file-change": {"anyOf": [{"type":"object","properties":{"operation":{"type":"string","enum":["create"]},"path":{"type":"string"},"content":{"type":"string"}},"required":["operation","path","content"],"additionalProperties":false},{"type":"object","properties":{"operation":{"type":"string","enum":["update"]},"path":{"type":"string"},"expected_content_digest":{"type":"string"},"content":{"type":"string"}},"required":["operation","path","expected_content_digest","content"],"additionalProperties":false},{"type":"object","properties":{"operation":{"type":"string","enum":["delete"]},"path":{"type":"string"},"expected_content_digest":{"type":"string"}},"required":["operation","path","expected_content_digest"],"additionalProperties":false}]},
    "writer-artifact": {"type":"object","properties":{"kind":{"type":"string","enum":["changes"]},"changes":{"type":"array","items":{"$ref":"#/$defs/file-change"}}},"required":["kind","changes"],"additionalProperties":false},
    "result": {"anyOf": [{"type":"object","properties":{"role":{"type":"string","enum":["writer"]},"artifact":{"$ref":"#/$defs/writer-artifact"}},"required":["role","artifact"],"additionalProperties":false},{"type": "object","properties": {"role": {"type": "string","enum": ["verifier"]},"subject_evidence": {"type": "array","items": {"$ref": "#/$defs/evidence"}},"requirement_results": {"type": "array","items": {"$ref": "#/$defs/requirement"}}},"required": ["role","subject_evidence","requirement_results"],"additionalProperties": false},{"type": "object","properties": {"role": {"type": "string","enum": ["reviewer"]},"summary": {"type": "string"},"subject_evidence": {"type": "array","items": {"$ref": "#/$defs/evidence"}},"requirement_results": {"type": "array","items": {"$ref": "#/$defs/requirement"}},"blocking_findings": {"type": "array","items": {"$ref": "#/$defs/observation"}},"improvements": {"type": "array","items": {"$ref": "#/$defs/observation"}},"learning_candidates": {"type": "array","items": {"$ref": "#/$defs/learning"}}},"required": ["role","summary","subject_evidence","requirement_results","blocking_findings","improvements","learning_candidates"],"additionalProperties": false}]},
    "request": {"type": "object","properties": {"reason": {"type": "string"},"blocked_verification_units": {"type": "array","items": {"$ref":"#/$defs/verification-unit"}},"subject_evidence": {"type": "array","items": {"$ref": "#/$defs/evidence"}},"candidate": {"anyOf": [{"type": "object","properties": {"kind": {"type": "string","enum": ["additional-workspace-target"]},"workspace_relative_path": {"type": "string"}},"required": ["kind","workspace_relative_path"],"additionalProperties": false},{"type": "object","properties": {"kind": {"type": "string","enum": ["vault-evidence"]},"repository_relative_path": {"type": "string"}},"required": ["kind","repository_relative_path"],"additionalProperties": false}]}},"required": ["reason","blocked_verification_units","subject_evidence","candidate"],"additionalProperties": false}
  }
}"##
}

pub(crate) fn codex_output_schema_for_role(
    prepared: &PreparedHarnessRun,
    role: HarnessRole,
) -> Result<String> {
    let assigned = prepared.plan.requirements.iter().any(|requirement| {
        matches!(requirement.owner, VerificationOwner::Role { role: owner } if owner == role)
    });
    let mut schema: serde_json::Value = decode_current_json(
        codex_output_schema().as_bytes(),
        "built-in Codex output schema",
    )
    .map_err(|_| ContextVaultError::invalid_input("invalid built-in Codex output schema"))?;
    let outcomes = schema["properties"]["outcome"]["anyOf"]
        .as_array_mut()
        .ok_or_else(|| ContextVaultError::invalid_input("invalid built-in outcome schema"))?;
    // Process cancellation and deadline expiry come from the supervisor, not a
    // model claim. Reviewer findings belong in the completed result so Core can
    // request a revision instead of halting the execution.
    // Core permits MissingContext only for a Reviewer with owned requirements.
    outcomes.retain(|branch| {
        let status = &branch["properties"]["status"]["enum"];
        status == &serde_json::json!(["completed"])
            || status == &serde_json::json!(["failed"])
            || status == &serde_json::json!(["unsupported"])
            || (role == HarnessRole::Reviewer
                && assigned
                && status == &serde_json::json!(["missing-context"]))
    });
    if role == HarnessRole::Reviewer {
        for branch in outcomes.iter_mut() {
            match branch["properties"]["status"]["enum"][0].as_str() {
                Some("completed") => {
                    branch["description"] = serde_json::json!(
                        "Use for a completed review, including correctable quality findings; report those in blocking_findings and requirement_results."
                    );
                }
                Some("failed") => {
                    branch["description"] = serde_json::json!(
                        "Use only when the review itself could not be performed; do not use for findings about a candidate."
                    );
                }
                Some("unsupported") => {
                    branch["description"] = serde_json::json!(
                        "Use only when the planned review cannot be completed and no valid MissingContext candidate can be named; do not use for correctable candidate findings."
                    );
                }
                _ => {}
            }
        }
    }
    if matches!(role, HarnessRole::Verifier | HarnessRole::Reviewer) {
        schema["$defs"]["requirement"]["properties"]["evidence"]["description"] = serde_json::json!(
            "For every requirement, include an evidence reference that binds the evaluated subject. When the invocation subject is produced-artifact, include source=produced-artifact and copy its exact artifact_digest; target and bound-document references alone are insufficient."
        );
    }
    let branches = schema["$defs"]["result"]["anyOf"]
        .as_array_mut()
        .ok_or_else(|| ContextVaultError::invalid_input("invalid built-in role schema"))?;
    branches.retain(|branch| branch["properties"]["role"]["enum"] == serde_json::json!([role]));
    let branch = branches
        .first_mut()
        .ok_or_else(|| ContextVaultError::invalid_input("missing built-in role schema"))?;
    if !assigned && matches!(role, HarnessRole::Verifier | HarnessRole::Reviewer) {
        branch["properties"]["requirement_results"] = serde_json::json!({
            "type": "array",
            "items": {"$ref": "#/$defs/requirement"},
            "description": "Return exactly []: this role has no assigned verification requirements. Do not invent requirement results."
        });
    }
    serde_json::to_string(&schema)
        .map_err(|_| ContextVaultError::invalid_input("cannot encode Codex role schema"))
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
            .map_err(|error| ContextVaultError::invalid_input(error.to_string()))?
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "ontology-role-{}-{nonce}-{}",
            process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|error| ContextVaultError::io("create private Codex cwd", &path, error))?;
        let mut directory = Self(path);
        directory.0 = directory.0.canonicalize().map_err(|error| {
            ContextVaultError::io("resolve private Codex cwd", &directory.0, error)
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
}

impl CodexNativeOutput {
    pub(crate) fn observe(
        &mut self,
        line: &[u8],
        now: u64,
    ) -> std::result::Result<(), &'static str> {
        let event: CodexEvent = decode_current_json(line, "native Codex event")
            .map_err(|_| "invalid or unsupported native Codex JSONL")?;
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
            CodexEvent::TurnCompleted { .. } => {
                if !self.turn_started || self.terminal.is_some() {
                    return Err("missing or duplicate native turn boundary");
                }
                self.terminal = Some((now, true));
            }
        }
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
        decode_current_json::<CodexResponse>(text.as_bytes(), "native Codex role response")
            .map(|response| response.outcome)
            .map_err(|_| codex_response_shape_error(text))
    }
}

fn codex_response_shape_error(text: &str) -> &'static str {
    let Ok(value) = decode_current_json::<serde_json::Value>(
        text.as_bytes(),
        "native Codex role response diagnostic",
    ) else {
        return "native Codex agent_message is not JSON";
    };
    let Some(outcome) = value.get("outcome").and_then(serde_json::Value::as_object) else {
        return "native Codex agent_message has no outcome object";
    };
    let Some(status) = outcome.get("status").and_then(serde_json::Value::as_str) else {
        return "native Codex outcome has no status string";
    };
    if status == "completed" {
        let Some(result) = outcome.get("result").and_then(serde_json::Value::as_object) else {
            return "native Codex completed outcome has no result object";
        };
        if result
            .get("role")
            .and_then(serde_json::Value::as_str)
            .is_none()
        {
            return "native Codex completed result has no role string";
        }
        if result.get("role").and_then(serde_json::Value::as_str) == Some("verifier") {
            let Some(evidence) = result
                .get("subject_evidence")
                .and_then(serde_json::Value::as_array)
            else {
                return "native Codex verifier has no subject evidence array";
            };
            if evidence.iter().any(|item| {
                serde_json::to_vec(item).ok().is_none_or(|bytes| {
                    decode_current_json::<ResultEvidenceReference>(
                        &bytes,
                        "native Codex verifier evidence",
                    )
                    .is_err()
                })
            }) {
                return "native Codex verifier has invalid subject evidence";
            }
            let Some(requirements) = result
                .get("requirement_results")
                .and_then(serde_json::Value::as_array)
            else {
                return "native Codex verifier has no requirement results array";
            };
            if requirements.iter().any(|item| {
                serde_json::to_vec(item).ok().is_none_or(|bytes| {
                    decode_current_json::<RequirementResult>(
                        &bytes,
                        "native Codex verifier requirement",
                    )
                    .is_err()
                })
            }) {
                return "native Codex verifier has invalid requirement result";
            }
        }
    }
    "native Codex role response has incompatible field types, values, or fields outside the exact schema"
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
) -> Result<RoleExecutionResult> {
    if !codex_role_supported(prepared, invocation.role) {
        return Err(ContextVaultError::invalid_input(
            "unsupported runtime: only DocumentWrite Writer and Reviewer/Verifier may execute",
        ));
    }
    // Serialize the exact typed segments once: no wrapper, prompt preamble, target
    // reread, summary, content replacement or argument-length-dependent transport.
    let input = serde_json::to_vec(&invocation.segments)
        .map_err(|error| ContextVaultError::invalid_input(error.to_string()))?;
    if input.len()
        > prepared
            .role_run
            .runtime_capabilities
            .max_role_invocation_bytes
        || input.len() as u64 > MAX_HARNESS_JSON_BYTES
    {
        return Err(ContextVaultError::invalid_input(
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
        let schema = codex_output_schema_for_role(prepared, invocation.role)?;
        let (lifecycle, outcome) =
            run_codex_process_with_schema(binary, &input, invocation.role, &limits, &schema)?;
        invocation
            .bind_result(lifecycle, outcome)
            .map_err(|error| ContextVaultError::invalid_input(error.to_string()))
    }
    #[cfg(not(unix))]
    {
        let _ = (binary, input);
        Err(ContextVaultError::invalid_input(
            "unsupported runtime: Codex requires Unix process groups",
        ))
    }
}

#[cfg(unix)]
pub(crate) struct CodexProcess {
    child: process::Child,
    group: i32,
    cleaned: bool,
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
            let _ = self.child.try_wait();
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

#[cfg(unix)]
#[cfg(test)]
pub(crate) fn run_codex_process(
    binary: &Path,
    input: &[u8],
    role: HarnessRole,
    limits: &RoleLifecycleLimits,
) -> Result<(ReportedRoleLifecycle, RoleExecutionOutcome)> {
    run_codex_process_with_schema(binary, input, role, limits, codex_output_schema())
}

#[cfg(unix)]
#[allow(
    clippy::too_many_lines,
    reason = "one bounded process supervisor owns stdin, native events, deadlines, group termination and reaping"
)]
fn run_codex_process_with_schema(
    binary: &Path,
    input: &[u8],
    role: HarnessRole,
    limits: &RoleLifecycleLimits,
    output_schema: &str,
) -> Result<(ReportedRoleLifecycle, RoleExecutionOutcome)> {
    use std::os::unix::{
        fs::OpenOptionsExt as _,
        process::{CommandExt as _, ExitStatusExt as _},
    };
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    let directory = CodexDirectory::create()?;
    let schema_path = directory.0.join("outcome-schema.json");
    let schema = output_schema.as_bytes();
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&schema_path)
        .and_then(|mut file| file.write_all(schema))
        .map_err(|error| ContextVaultError::io("write Codex output schema", &schema_path, error))?;
    let started_at_millis = u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| ContextVaultError::invalid_input(error.to_string()))?
            .as_millis(),
    )
    .map_err(|_| ContextVaultError::invalid_input("Codex start time overflow"))?;
    let started = Instant::now();
    // Reserve one polling quantum for scheduling and at most half the close
    // budget for forced group termination/reaping. All reported times are actual
    // observations; an OS overrun fails rather than being clamped into acceptance.
    let quantum = Duration::from_millis(2);
    let total = Duration::from_millis(limits.max_total_role_millis);
    let close = Duration::from_millis(limits.max_role_close_millis);
    let grace = Duration::from_millis(limits.max_role_grace_millis);
    let execution = Duration::from_millis(limits.max_role_execution_millis)
        .min(total.saturating_sub(close))
        .saturating_sub(quantum);
    let total_deadline = started
        .checked_add(total)
        .ok_or_else(|| ContextVaultError::invalid_input("Codex total deadline overflow"))?;
    let execution_deadline = started
        .checked_add(execution)
        .ok_or_else(|| ContextVaultError::invalid_input("Codex execution deadline overflow"))?;
    if execution.is_zero()
        || close.is_zero()
        || started_at_millis
            .checked_add(limits.max_total_role_millis)
            .is_none()
    {
        return Err(ContextVaultError::invalid_input(
            "unsupported runtime: insufficient finite Codex lifecycle budget",
        ));
    }
    let child = process::Command::new(binary)
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
        .arg(&schema_path)
        .arg("-")
        .current_dir(&directory.0)
        .process_group(0)
        .stdin(process::Stdio::piped())
        .stdout(process::Stdio::piped())
        .stderr(process::Stdio::piped())
        .spawn()
        .map_err(|error| ContextVaultError::io("start Codex role", binary, error))?;
    let group =
        i32::try_from(child.id()).expect("a Unix child PID must fit the native signed pid_t");
    let mut process = CodexProcess {
        child,
        group,
        cleaned: false,
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
    let mut native = CodexNativeOutput::default();
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
    let now_millis = || {
        started_at_millis
            .saturating_add(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX))
    };
    loop {
        let now = Instant::now();
        if now >= total_deadline {
            return Err(ContextVaultError::invalid_input(
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
                                failure = native
                                    .observe(&pending[consumed..index], now_millis())
                                    .err();
                            }
                            consumed = index + 1;
                        }
                    }
                    pending.drain(..consumed);
                    if count == 0 {
                        if !pending.is_empty() && failure.is_none() {
                            failure = native.observe(&pending, now_millis()).err();
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
            if let Some((at, _)) = native.terminal {
                terminal = Some(at);
            } else if failure.is_some() || (status.is_some() && stdout.is_none()) {
                terminal = Some(now_millis());
            } else if now >= execution_deadline {
                timed_out = true;
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
            && (status.is_some() || now >= deadline.checked_sub(quantum).unwrap_or(deadline))
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
                || (timed_out && terminal.is_some())
                || kill_deadline.is_some_and(|deadline| now >= deadline))
        {
            if status.is_none() && failure.is_none() && !timed_out {
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
                return Err(ContextVaultError::invalid_input(codex_failure_message(
                    &detail,
                    exit,
                    &stderr_tail,
                )));
            }
            process.cleaned = true;
            let Some((context_id, context_ready_at_millis)) = native.context.clone() else {
                return Err(ContextVaultError::invalid_input(codex_failure_message(
                    "Codex execution failed without an observed native thread.started context ID",
                    exit,
                    &stderr_tail,
                )));
            };
            let outcome = if timed_out {
                RoleExecutionOutcome::TimedOut
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
                native.outcome(exit.success()).unwrap_or_else(|message| {
                    RoleExecutionOutcome::Failed {
                        message: codex_failure_message(message, exit, &stderr_tail),
                    }
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
                first_output_at_millis: native.first_output,
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
                return Err(ContextVaultError::invalid_input(
                    "observed Codex lifecycle exceeded prepared limits",
                ));
            }
            return Ok((lifecycle, outcome));
        }
        if close_deadline.is_some_and(|deadline| now >= deadline) {
            return Err(ContextVaultError::invalid_input(
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
