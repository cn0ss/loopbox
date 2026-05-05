use super::{
    build_notification_line, build_request_line, build_response_line, item_to_transcript,
    parse_jsonrpc_line, CodexAgentsSettings, CodexInbound, CodexTranscriptItem,
    CodexTranscriptState, LoopboxConfig,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

const STDERR_TAIL_LIMIT: usize = 80;
const EVENT_TAIL_LIMIT: usize = 120;
const CLIENT_NAME: &str = "loopbox_codex_agents";
const LOOPBOX_MCP_SERVER_NAME: &str = "loopbox";
const REQUIRED_LOOPBOX_CREATION_TOOLS: &[&str] =
    &["loopbox_validate_project_config", "loopbox_create_project"];

static CODEX_SESSION: OnceLock<Mutex<Option<Arc<CodexAppInner>>>> = OnceLock::new();
static PREFILLED_PROMPT: OnceLock<Mutex<Option<String>>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexAgentModel {
    pub id: String,
    pub display_name: String,
    pub default_effort: Option<String>,
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexAgentThreadSummary {
    pub id: String,
    pub title: String,
    pub preview: String,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexAgentPendingRequest {
    pub request_id: String,
    pub item_id: Option<String>,
    pub method: String,
    pub title: String,
    pub body: String,
    pub raw_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexAgentAuthState {
    pub label: String,
    pub requires_auth: bool,
    pub signed_in: bool,
}

impl Default for CodexAgentAuthState {
    fn default() -> Self {
        Self {
            label: "Not checked".to_string(),
            requires_auth: false,
            signed_in: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexAgentsSnapshot {
    pub enabled: bool,
    pub running: bool,
    pub starting: bool,
    pub codex_binary: String,
    pub active_thread_id: Option<String>,
    pub active_turn_id: Option<String>,
    pub turn_status: Option<String>,
    pub auth: CodexAgentAuthState,
    pub models: Vec<CodexAgentModel>,
    pub threads: Vec<CodexAgentThreadSummary>,
    pub loopbox_mcp_tools: Vec<String>,
    pub loopbox_mcp_missing_tools: Vec<String>,
    pub transcript: Vec<CodexTranscriptItem>,
    pub pending_requests: Vec<CodexAgentPendingRequest>,
    pub errors: Vec<String>,
    pub stderr_tail: Vec<String>,
    pub event_log: Vec<String>,
    pub prefilled_prompt: Option<String>,
}

impl Default for CodexAgentsSnapshot {
    fn default() -> Self {
        Self {
            enabled: true,
            running: false,
            starting: false,
            codex_binary: "codex".to_string(),
            active_thread_id: None,
            active_turn_id: None,
            turn_status: None,
            auth: CodexAgentAuthState::default(),
            models: Vec::new(),
            threads: Vec::new(),
            loopbox_mcp_tools: Vec::new(),
            loopbox_mcp_missing_tools: Vec::new(),
            transcript: Vec::new(),
            pending_requests: Vec::new(),
            errors: Vec::new(),
            stderr_tail: Vec::new(),
            event_log: Vec::new(),
            prefilled_prompt: take_prefilled_prompt(),
        }
    }
}

#[derive(Debug, Clone)]
struct PendingTurn {
    input: String,
}

#[derive(Debug)]
struct CodexAppState {
    enabled: bool,
    running: bool,
    starting: bool,
    codex_binary: String,
    active_thread_id: Option<String>,
    active_turn_id: Option<String>,
    auth: CodexAgentAuthState,
    models: Vec<CodexAgentModel>,
    threads: Vec<CodexAgentThreadSummary>,
    loopbox_mcp_tools: Vec<String>,
    loopbox_mcp_missing_tools: Vec<String>,
    transcript: CodexTranscriptState,
    pending_requests: Vec<CodexAgentPendingRequest>,
    outbound_methods: BTreeMap<u64, String>,
    pending_initial_turn: Option<PendingTurn>,
    errors: Vec<String>,
    stderr_tail: VecDeque<String>,
    event_log: VecDeque<String>,
}

impl CodexAppState {
    fn new(config: &LoopboxConfig) -> Self {
        let codex_binary = codex_binary(config);
        Self {
            enabled: config.global.codex_agents.enabled,
            running: false,
            starting: true,
            codex_binary,
            active_thread_id: None,
            active_turn_id: None,
            auth: CodexAgentAuthState::default(),
            models: Vec::new(),
            threads: Vec::new(),
            loopbox_mcp_tools: Vec::new(),
            loopbox_mcp_missing_tools: Vec::new(),
            transcript: CodexTranscriptState::default(),
            pending_requests: Vec::new(),
            outbound_methods: BTreeMap::new(),
            pending_initial_turn: None,
            errors: Vec::new(),
            stderr_tail: VecDeque::new(),
            event_log: VecDeque::new(),
        }
    }

    fn snapshot(&self) -> CodexAgentsSnapshot {
        CodexAgentsSnapshot {
            enabled: self.enabled,
            running: self.running,
            starting: self.starting,
            codex_binary: self.codex_binary.clone(),
            active_thread_id: self.active_thread_id.clone(),
            active_turn_id: self.active_turn_id.clone(),
            turn_status: self.transcript.turn_status.clone(),
            auth: self.auth.clone(),
            models: self.models.clone(),
            threads: self.threads.clone(),
            loopbox_mcp_tools: self.loopbox_mcp_tools.clone(),
            loopbox_mcp_missing_tools: self.loopbox_mcp_missing_tools.clone(),
            transcript: self.transcript.items.clone(),
            pending_requests: self.pending_requests.clone(),
            errors: self.errors.clone(),
            stderr_tail: self.stderr_tail.iter().cloned().collect(),
            event_log: self.event_log.iter().cloned().collect(),
            prefilled_prompt: take_prefilled_prompt(),
        }
    }

    fn push_event(&mut self, event: impl Into<String>) {
        push_limited(&mut self.event_log, event.into(), EVENT_TAIL_LIMIT);
    }

    fn push_error(&mut self, error: impl Into<String>) {
        let error = error.into();
        self.errors.push(error.clone());
        self.push_event(format!("error: {error}"));
    }

    fn push_optimistic_user_message(&mut self, text: &str) {
        self.transcript.items.push(CodexTranscriptItem {
            id: format!("optimistic-user-{}", unix_time_ms()),
            kind: "userMessage".to_string(),
            title: "You".to_string(),
            text: text.to_string(),
            status: "sending".to_string(),
            raw_json: String::new(),
        });
        self.transcript.turn_status = Some("inProgress".to_string());
    }
}

struct CodexAppInner {
    state: Mutex<CodexAppState>,
    settings: Mutex<CodexAgentsSettings>,
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    next_id: AtomicU64,
}

pub fn codex_agents_snapshot(config: &LoopboxConfig) -> CodexAgentsSnapshot {
    let session = session_lock().lock().ok().and_then(|guard| guard.clone());
    if let Some(session) = session {
        if let Ok(mut state) = session.state.lock() {
            state.enabled = config.global.codex_agents.enabled;
            return state.snapshot();
        }
    }

    CodexAgentsSnapshot {
        enabled: config.global.codex_agents.enabled,
        codex_binary: codex_binary(config),
        ..CodexAgentsSnapshot::default()
    }
}

pub fn codex_agents_prefill_prompt(prompt: impl Into<String>) {
    if let Ok(mut slot) = prefilled_prompt_lock().lock() {
        *slot = Some(prompt.into());
    }
}

pub fn codex_agents_start(config: &LoopboxConfig) -> Result<(), String> {
    if !config.global.codex_agents.enabled {
        return Err("Codex Agents are disabled in Loopbox config.".to_string());
    }

    let lock = session_lock();
    let mut guard = lock
        .lock()
        .map_err(|_| "Codex session lock poisoned.".to_string())?;
    if let Some(existing) = guard.as_ref() {
        let running = existing
            .state
            .lock()
            .map(|state| state.running || state.starting)
            .unwrap_or(false);
        if running {
            if let Ok(mut settings) = existing.settings.lock() {
                *settings = config.global.codex_agents.clone();
            }
            return Ok(());
        }
    }

    let binary = codex_binary(config);
    let mut command = Command::new(&binary);
    command
        .arg("app-server")
        .arg("--listen")
        .arg("stdio://")
        .arg("-c")
        .arg(format!(
            "mcp_servers.loopbox.command={}",
            toml_string(&current_exe_path()?)
        ))
        .arg("-c")
        .arg(r#"mcp_servers.loopbox.args=["__loopbox_mcp_server"]"#)
        .arg("-c")
        .arg("mcp_servers.loopbox.required=true")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("NO_COLOR", "1")
        .env("RUST_LOG", "warn");

    let mut child = command
        .spawn()
        .map_err(|err| format!("Failed to start `{binary} app-server`: {err}"))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Codex app-server stdin was not available.".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Codex app-server stdout was not available.".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Codex app-server stderr was not available.".to_string())?;

    let inner = Arc::new(CodexAppInner {
        state: Mutex::new(CodexAppState::new(config)),
        settings: Mutex::new(config.global.codex_agents.clone()),
        child: Mutex::new(child),
        stdin: Mutex::new(stdin),
        next_id: AtomicU64::new(1),
    });

    spawn_stdout_reader(inner.clone(), stdout);
    spawn_stderr_reader(inner.clone(), stderr);
    *guard = Some(inner.clone());
    drop(guard);

    send_initialize(&inner)?;
    Ok(())
}

pub fn codex_agents_stop() -> Result<(), String> {
    let session = session_lock()
        .lock()
        .map_err(|_| "Codex session lock poisoned.".to_string())?
        .take();
    if let Some(session) = session {
        if let Ok(mut child) = session.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Ok(mut state) = session.state.lock() {
            state.running = false;
            state.starting = false;
            state.push_event("stopped Codex app-server");
        }
    }
    Ok(())
}

pub fn codex_agents_send_message(
    config: &LoopboxConfig,
    _selected_project: Option<String>,
    input: String,
) -> Result<(), String> {
    let input = input.trim().to_string();
    if input.is_empty() {
        return Err("Message cannot be empty.".to_string());
    }

    codex_agents_start(config)?;
    let session = active_session()?;
    if let Ok(mut settings) = session.settings.lock() {
        *settings = config.global.codex_agents.clone();
    }
    let turn_input = input.clone();

    let active_thread = session
        .state
        .lock()
        .map_err(|_| "Codex state lock poisoned.".to_string())?
        .active_thread_id
        .clone();

    if let Some(thread_id) = active_thread {
        {
            let mut state = session
                .state
                .lock()
                .map_err(|_| "Codex state lock poisoned.".to_string())?;
            state.push_optimistic_user_message(&input);
        }
        send_turn_start(&session, thread_id, turn_input)?;
    } else {
        {
            let mut state = session
                .state
                .lock()
                .map_err(|_| "Codex state lock poisoned.".to_string())?;
            if state.pending_initial_turn.is_some() {
                return Err("Codex is still creating the first thread.".to_string());
            }
            state.push_optimistic_user_message(&input);
            state.pending_initial_turn = Some(PendingTurn { input: turn_input });
        }
        send_thread_start(&session)?;
    }

    Ok(())
}

pub fn codex_agents_new_chat(config: &LoopboxConfig) -> Result<(), String> {
    codex_agents_start(config)?;
    let session = active_session()?;
    if let Ok(mut settings) = session.settings.lock() {
        *settings = config.global.codex_agents.clone();
    }
    {
        let mut state = session
            .state
            .lock()
            .map_err(|_| "Codex state lock poisoned.".to_string())?;
        state.active_thread_id = None;
        state.active_turn_id = None;
        state.transcript = CodexTranscriptState::default();
        state.pending_initial_turn = None;
        state.pending_requests.clear();
        state.push_event("started a new unsaved Codex chat");
    }
    send_thread_list(&session)?;
    Ok(())
}

pub fn codex_agents_reload_tools(config: &LoopboxConfig) -> Result<(), String> {
    codex_agents_start(config)?;
    let session = active_session()?;
    if let Ok(mut settings) = session.settings.lock() {
        *settings = config.global.codex_agents.clone();
    }
    send_request(&session, "config/mcpServer/reload", json!({}))?;
    Ok(())
}

pub fn codex_agents_resume_thread(config: &LoopboxConfig, thread_id: &str) -> Result<(), String> {
    let thread_id = thread_id.trim().to_string();
    if thread_id.is_empty() {
        return Err("Thread id cannot be empty.".to_string());
    }
    codex_agents_start(config)?;
    let session = active_session()?;
    if let Ok(mut settings) = session.settings.lock() {
        *settings = config.global.codex_agents.clone();
    }
    {
        let mut state = session
            .state
            .lock()
            .map_err(|_| "Codex state lock poisoned.".to_string())?;
        state.active_thread_id = Some(thread_id.clone());
        state.active_turn_id = None;
        state.transcript = CodexTranscriptState {
            turn_status: Some("loading".to_string()),
            ..CodexTranscriptState::default()
        };
        state.pending_initial_turn = None;
        state.pending_requests.clear();
        state.push_event(format!("loading Codex thread: {thread_id}"));
    }
    send_request(
        &session,
        "thread/read",
        json!({ "threadId": thread_id, "includeTurns": true }),
    )?;
    send_request(&session, "thread/resume", thread_resume_params(&thread_id))?;
    Ok(())
}

pub fn codex_agents_interrupt_turn() -> Result<(), String> {
    let session = active_session()?;
    let (thread_id, turn_id) = {
        let state = session
            .state
            .lock()
            .map_err(|_| "Codex state lock poisoned.".to_string())?;
        (state.active_thread_id.clone(), state.active_turn_id.clone())
    };
    let thread_id = thread_id.ok_or_else(|| "No active Codex thread.".to_string())?;
    let turn_id = turn_id.ok_or_else(|| "No active Codex turn.".to_string())?;
    send_request(
        &session,
        "turn/interrupt",
        json!({
            "threadId": thread_id,
            "turnId": turn_id,
        }),
    )?;
    Ok(())
}

pub fn codex_agents_accept_request(request_id: &str) -> Result<(), String> {
    respond_to_server_request(request_id, "accept")
}

pub fn codex_agents_decline_request(request_id: &str) -> Result<(), String> {
    respond_to_server_request(request_id, "decline")
}

fn respond_to_server_request(request_id: &str, action: &str) -> Result<(), String> {
    let session = active_session()?;
    let pending = {
        let mut state = session
            .state
            .lock()
            .map_err(|_| "Codex state lock poisoned.".to_string())?;
        let Some(index) = state
            .pending_requests
            .iter()
            .position(|request| request.request_id == request_id)
        else {
            return Err("Pending Codex request was not found.".to_string());
        };
        state.pending_requests.remove(index)
    };

    let id: Value = serde_json::from_str(&pending.request_id)
        .map_err(|err| format!("Pending Codex request id is invalid: {err}"))?;
    let result = match pending.method.as_str() {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            json!({ "decision": action })
        }
        "mcpServer/elicitation/request" if action == "accept" => {
            json!({ "action": "accept", "content": { "confirmed": true } })
        }
        "mcpServer/elicitation/request" => json!({ "action": action }),
        _ => json!({}),
    };
    send_response(&session, &id, result)?;
    Ok(())
}

fn send_initialize(inner: &Arc<CodexAppInner>) -> Result<(), String> {
    send_request(
        inner,
        "initialize",
        json!({
            "clientInfo": {
                "name": CLIENT_NAME,
                "title": "Loopbox Codex Agents",
                "version": env!("CARGO_PKG_VERSION"),
            },
            "capabilities": {
                "experimentalApi": true
            }
        }),
    )?;
    send_notification(inner, "initialized", json!({}))?;
    send_request(inner, "account/read", json!({ "refreshToken": false }))?;
    send_request(
        inner,
        "model/list",
        json!({ "limit": 50, "includeHidden": false }),
    )?;
    send_thread_list(inner)?;
    send_mcp_server_status_list(inner)?;

    if let Ok(mut state) = inner.state.lock() {
        state.running = true;
        state.starting = false;
        state.push_event("initialized Codex app-server");
    }
    Ok(())
}

fn send_thread_start(inner: &Arc<CodexAppInner>) -> Result<(), String> {
    let settings = inner
        .settings
        .lock()
        .map_err(|_| "Codex settings lock poisoned.".to_string())?
        .clone();
    let mut params = json!({
        "developerInstructions": loopbox_developer_instructions(),
        "serviceName": "loopbox_agents"
    });
    if !settings.default_model.trim().is_empty() {
        params["model"] = json!(settings.default_model.trim());
    }
    if let Some(sandbox) = codex_sandbox_wire_value(&settings.default_sandbox) {
        params["sandbox"] = json!(sandbox);
    }
    send_request(inner, "thread/start", params).map(|_| ())
}

fn send_turn_start(
    inner: &Arc<CodexAppInner>,
    thread_id: String,
    input: String,
) -> Result<(), String> {
    let settings = inner
        .settings
        .lock()
        .map_err(|_| "Codex settings lock poisoned.".to_string())?
        .clone();
    let mut params = json!({
        "threadId": thread_id,
        "input": [{ "type": "text", "text": input }],
    });
    if !settings.default_effort.trim().is_empty() {
        params["effort"] = json!(settings.default_effort.trim());
    }
    send_request(inner, "turn/start", params).map(|_| ())
}

fn send_thread_list(inner: &Arc<CodexAppInner>) -> Result<u64, String> {
    send_request(
        inner,
        "thread/list",
        json!({
            "limit": 50,
            "sortKey": "updated_at",
            "sourceKinds": ["cli", "vscode", "appServer"]
        }),
    )
}

fn send_mcp_server_status_list(inner: &Arc<CodexAppInner>) -> Result<u64, String> {
    send_request(
        inner,
        "mcpServerStatus/list",
        json!({
            "limit": 50,
            "detail": "toolsAndAuthOnly"
        }),
    )
}

fn send_request(inner: &Arc<CodexAppInner>, method: &str, params: Value) -> Result<u64, String> {
    let id = inner.next_id.fetch_add(1, Ordering::Relaxed);
    let line = build_request_line(id, method, params)?;
    write_line(inner, &line)?;
    if let Ok(mut state) = inner.state.lock() {
        state.outbound_methods.insert(id, method.to_string());
        state.push_event(format!("request {id}: {method}"));
    }
    Ok(id)
}

fn send_notification(
    inner: &Arc<CodexAppInner>,
    method: &str,
    params: Value,
) -> Result<(), String> {
    let line = build_notification_line(method, params)?;
    write_line(inner, &line)?;
    if let Ok(mut state) = inner.state.lock() {
        state.push_event(format!("notify: {method}"));
    }
    Ok(())
}

fn send_response(inner: &Arc<CodexAppInner>, id: &Value, result: Value) -> Result<(), String> {
    let line = build_response_line(id, result)?;
    write_line(inner, &line)?;
    if let Ok(mut state) = inner.state.lock() {
        state.push_event(format!("responded to server request {id}"));
    }
    Ok(())
}

fn write_line(inner: &Arc<CodexAppInner>, line: &str) -> Result<(), String> {
    let mut stdin = inner
        .stdin
        .lock()
        .map_err(|_| "Codex stdin lock poisoned.".to_string())?;
    stdin
        .write_all(line.as_bytes())
        .and_then(|_| stdin.write_all(b"\n"))
        .and_then(|_| stdin.flush())
        .map_err(|err| format!("Failed to write to Codex app-server: {err}"))
}

fn spawn_stdout_reader(inner: Arc<CodexAppInner>, stdout: impl std::io::Read + Send + 'static) {
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            match line {
                Ok(line) if line.trim().is_empty() => {}
                Ok(line) => match parse_jsonrpc_line(&line) {
                    Ok(inbound) => handle_inbound(&inner, inbound),
                    Err(err) => {
                        if let Ok(mut state) = inner.state.lock() {
                            state.push_error(err);
                        }
                    }
                },
                Err(err) => {
                    if let Ok(mut state) = inner.state.lock() {
                        state.push_error(format!("Codex stdout read failed: {err}"));
                    }
                    break;
                }
            }
        }
        if let Ok(mut state) = inner.state.lock() {
            state.running = false;
            state.starting = false;
            state.push_event("Codex app-server stdout closed");
        }
    });
}

fn spawn_stderr_reader(inner: Arc<CodexAppInner>, stderr: impl std::io::Read + Send + 'static) {
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for raw_line in reader.lines().map_while(Result::ok) {
            let line = strip_ansi(&raw_line);
            if !should_keep_stderr_line(&line) {
                continue;
            }
            if let Ok(mut state) = inner.state.lock() {
                push_limited(&mut state.stderr_tail, line, STDERR_TAIL_LIMIT);
            }
        }
    });
}

fn handle_inbound(inner: &Arc<CodexAppInner>, inbound: CodexInbound) {
    let actions = match &inbound {
        CodexInbound::Response { id, result, error } => {
            handle_response(inner, id, result.as_ref(), error.as_ref())
        }
        CodexInbound::Notification { method, params } => {
            let refresh_threads = method == "turn/completed" || method == "thread/started";
            handle_notification(inner, method, params);
            if refresh_threads {
                let _ = send_thread_list(inner);
            }
            PostResponseActions::default()
        }
        CodexInbound::ServerRequest { id, method, params } => {
            handle_server_request(inner, id, method, params);
            PostResponseActions::default()
        }
    };

    if actions.refresh_mcp_status {
        if let Err(err) = send_mcp_server_status_list(inner) {
            if let Ok(mut state) = inner.state.lock() {
                state.push_error(err);
            }
        }
    }

    if let Some((thread_id, pending_turn)) = actions.pending_turn {
        if let Err(err) = send_turn_start(inner, thread_id, pending_turn.input) {
            if let Ok(mut state) = inner.state.lock() {
                state.push_error(err);
            }
        }
    }
}

#[derive(Default)]
struct PostResponseActions {
    pending_turn: Option<(String, PendingTurn)>,
    refresh_mcp_status: bool,
}

fn handle_response(
    inner: &Arc<CodexAppInner>,
    id: &Value,
    result: Option<&Value>,
    error: Option<&super::CodexRpcError>,
) -> PostResponseActions {
    let mut actions = PostResponseActions::default();
    let Ok(mut state) = inner.state.lock() else {
        return actions;
    };
    let method = id
        .as_u64()
        .and_then(|id| state.outbound_methods.remove(&id))
        .unwrap_or_else(|| "<unknown>".to_string());

    if let Some(error) = error {
        state.push_error(format!("{method} failed: {}", error.message));
        return actions;
    }

    match method.as_str() {
        "account/read" => {
            if let Some(result) = result {
                state.auth = parse_auth_state(result);
            }
        }
        "model/list" => {
            state.models = result
                .and_then(|value| value.get("data"))
                .and_then(Value::as_array)
                .map(|models| models.iter().filter_map(parse_model).collect())
                .unwrap_or_default();
        }
        "thread/list" => {
            state.threads = result
                .and_then(|value| value.get("data"))
                .and_then(Value::as_array)
                .map(|threads| threads.iter().filter_map(parse_thread_summary).collect())
                .unwrap_or_default();
            let count = state.threads.len();
            state.push_event(format!("loaded {count} stored Codex thread(s)"));
        }
        "mcpServerStatus/list" => {
            let tools = result.map(parse_loopbox_mcp_tools).unwrap_or_default();
            let missing_tools = missing_loopbox_creation_tools(&tools);
            let count = tools.len();
            if missing_tools.is_empty() {
                state.push_event(format!("loaded {count} Loopbox MCP tool(s)"));
            } else {
                state.push_event(format!(
                    "Loopbox MCP missing tool(s): {}",
                    missing_tools.join(", ")
                ));
            }
            state.loopbox_mcp_tools = tools;
            state.loopbox_mcp_missing_tools = missing_tools;
        }
        "thread/read" => {
            if let Some(result) = result {
                let thread_id = result.pointer("/thread/id").and_then(Value::as_str);
                if thread_id == state.active_thread_id.as_deref() {
                    state.transcript = parse_thread_transcript(result);
                    state.transcript.turn_status = Some("idle".to_string());
                }
            }
        }
        "thread/start" | "thread/resume" => {
            if let Some(thread_id) = result
                .and_then(|value| value.pointer("/thread/id"))
                .and_then(Value::as_str)
            {
                state.active_thread_id = Some(thread_id.to_string());
                state.push_event(format!("active thread: {thread_id}"));
                if let Some(turn) = state.pending_initial_turn.take() {
                    actions.pending_turn = Some((thread_id.to_string(), turn));
                }
            }
        }
        "turn/start" | "review/start" => {
            if let Some(turn_id) = result
                .and_then(|value| value.pointer("/turn/id"))
                .and_then(Value::as_str)
            {
                state.active_turn_id = Some(turn_id.to_string());
                state.transcript.turn_status = Some("inProgress".to_string());
            }
        }
        "turn/interrupt" => {
            state.transcript.turn_status = Some("interrupted".to_string());
        }
        "config/mcpServer/reload" => {
            state.push_event("reloaded MCP server configuration");
            actions.refresh_mcp_status = true;
        }
        _ => {}
    }

    state.push_event(format!("response: {method}"));
    actions
}

fn handle_notification(inner: &Arc<CodexAppInner>, method: &str, params: &Value) {
    let Ok(mut state) = inner.state.lock() else {
        return;
    };
    if notification_item_type(params) == Some("userMessage") {
        state
            .transcript
            .items
            .retain(|item| !item.id.starts_with("optimistic-user-"));
    }
    state.transcript.apply_notification(method, params);

    match method {
        "turn/started" => {
            if let Some(turn_id) = params.pointer("/turn/id").and_then(Value::as_str) {
                state.active_turn_id = Some(turn_id.to_string());
            }
        }
        "turn/completed" => {
            state.active_turn_id = None;
        }
        "thread/status/changed" => {
            if let Some(thread_id) = params.get("threadId").and_then(Value::as_str) {
                state
                    .active_thread_id
                    .get_or_insert_with(|| thread_id.to_string());
            }
        }
        "account/updated" => {
            let auth_mode = params.get("authMode").and_then(Value::as_str);
            let plan = params.get("planType").and_then(Value::as_str);
            state.auth = CodexAgentAuthState {
                label: match (auth_mode, plan) {
                    (Some(mode), Some(plan)) => format!("{mode} ({plan})"),
                    (Some(mode), None) => mode.to_string(),
                    (None, _) => "Signed out".to_string(),
                },
                requires_auth: true,
                signed_in: auth_mode.is_some(),
            };
        }
        _ => {}
    }
    if let Some(message) = notification_error_message(method, params) {
        state.push_error(message);
    }
    state.push_event(format!("event: {method}"));
}

fn handle_server_request(inner: &Arc<CodexAppInner>, id: &Value, method: &str, params: &Value) {
    let Ok(mut state) = inner.state.lock() else {
        return;
    };

    let item_id = server_request_item_id(method, params, &state.transcript.items);
    let pending = CodexAgentPendingRequest {
        request_id: serde_json::to_string(id).unwrap_or_else(|_| id.to_string()),
        item_id,
        method: method.to_string(),
        title: server_request_title(method, params),
        body: server_request_body(method, params),
        raw_json: super::codex_protocol::pretty_json(params),
    };

    state.pending_requests.push(pending);
    state.push_event(format!("server request: {method}"));
}

fn server_request_item_id(
    method: &str,
    params: &Value,
    transcript: &[CodexTranscriptItem],
) -> Option<String> {
    params
        .get("itemId")
        .or_else(|| params.get("item_id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            let expected_kinds: &[&str] = match method {
                "item/commandExecution/requestApproval" => &["commandExecution"],
                "item/fileChange/requestApproval" => &["fileChange"],
                "mcpServer/elicitation/request" => &["mcpToolCall", "dynamicToolCall"],
                "item/tool/requestUserInput" => &["mcpToolCall", "dynamicToolCall"],
                _ => &[],
            };
            transcript
                .iter()
                .rev()
                .find(|item| {
                    expected_kinds.contains(&item.kind.as_str())
                        && !matches!(item.status.as_str(), "completed" | "failed" | "declined")
                })
                .map(|item| item.id.clone())
        })
}

fn parse_auth_state(result: &Value) -> CodexAgentAuthState {
    let requires_auth = result
        .get("requiresOpenaiAuth")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let Some(account) = result.get("account").filter(|value| !value.is_null()) else {
        return CodexAgentAuthState {
            label: if requires_auth {
                "Sign in with Codex CLI".to_string()
            } else {
                "No OpenAI auth required".to_string()
            },
            requires_auth,
            signed_in: !requires_auth,
        };
    };

    let account_type = account
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("account");
    let label = match account_type {
        "apiKey" => "API key".to_string(),
        "chatgpt" => {
            let email = account
                .get("email")
                .and_then(Value::as_str)
                .unwrap_or("ChatGPT");
            let plan = account.get("planType").and_then(Value::as_str);
            match plan {
                Some(plan) => format!("{email} ({plan})"),
                None => email.to_string(),
            }
        }
        other => other.to_string(),
    };

    CodexAgentAuthState {
        label,
        requires_auth,
        signed_in: true,
    }
}

fn parse_model(value: &Value) -> Option<CodexAgentModel> {
    let id = value
        .get("id")
        .or_else(|| value.get("model"))
        .and_then(Value::as_str)?
        .to_string();
    Some(CodexAgentModel {
        display_name: value
            .get("displayName")
            .and_then(Value::as_str)
            .unwrap_or(id.as_str())
            .to_string(),
        default_effort: value
            .get("defaultReasoningEffort")
            .and_then(Value::as_str)
            .map(str::to_string),
        is_default: value
            .get("isDefault")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        id,
    })
}

fn parse_thread_summary(value: &Value) -> Option<CodexAgentThreadSummary> {
    let id = value.get("id").and_then(Value::as_str)?.to_string();
    let preview = value
        .get("preview")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let title = if !name.is_empty() {
        name.to_string()
    } else if !preview.is_empty() {
        preview
            .chars()
            .take(64)
            .collect::<String>()
            .trim()
            .to_string()
    } else {
        "Untitled chat".to_string()
    };
    Some(CodexAgentThreadSummary {
        id,
        title,
        preview,
        created_at: value.get("createdAt").and_then(timestamp_value),
        updated_at: value.get("updatedAt").and_then(timestamp_value),
    })
}

fn parse_thread_transcript(result: &Value) -> CodexTranscriptState {
    let mut state = CodexTranscriptState::default();
    if let Some(items) = result.pointer("/thread/items").and_then(Value::as_array) {
        for item in items {
            state.items.push(item_to_transcript(item));
        }
    }
    if let Some(turns) = result.pointer("/thread/turns").and_then(Value::as_array) {
        for turn in turns {
            if let Some(items) = turn.get("items").and_then(Value::as_array) {
                for item in items {
                    state.items.push(item_to_transcript(item));
                }
            }
        }
    }
    state
}

fn timestamp_value(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        value
            .as_u64()
            .and_then(|timestamp| i64::try_from(timestamp).ok())
    })
}

fn server_request_title(method: &str, _params: &Value) -> String {
    match method {
        "mcpServer/elicitation/request" => "Approve Loopbox action".to_string(),
        "item/commandExecution/requestApproval" => "Approve command".to_string(),
        "item/fileChange/requestApproval" => "Approve file changes".to_string(),
        "item/tool/requestUserInput" => "Codex needs input".to_string(),
        _ => method.to_string(),
    }
}

fn server_request_body(method: &str, params: &Value) -> String {
    match method {
        "mcpServer/elicitation/request" => params
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("A Loopbox MCP tool is requesting approval.")
            .to_string(),
        "item/commandExecution/requestApproval" => {
            let reason = params.get("reason").and_then(Value::as_str).unwrap_or("");
            let command = params
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("<command unavailable>");
            let cwd = params.get("cwd").and_then(Value::as_str).unwrap_or("");
            format!("{reason}\n\n$ {command}\n{cwd}")
        }
        "item/fileChange/requestApproval" => params
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("Codex wants to apply file changes.")
            .to_string(),
        _ => super::codex_protocol::pretty_json(params),
    }
}

fn loopbox_developer_instructions() -> String {
    r#"You are embedded in Loopbox. Use the Loopbox MCP tools for sandbox, runtime, log, request, and resource questions instead of guessing local ports or reading Loopbox config by hand.

Loopbox vocabulary:
- In Loopbox, "sandbox" and "project" mean the same thing.
- You can create Loopbox sandboxes/projects when the MCP tools expose `loopbox_validate_project_config` and `loopbox_create_project`.

Operational rules:
- Prefer Loopbox project hostnames from tool output over guessed localhost ports.
- Fetch logs with explicit limits and ask before broad or expensive log reads.
- Use runtime/resource/request tools before suggesting fixes for failed services.
- To create a sandbox/project, first collect the required fields: sandbox name, absolute project directory, services, commands, working directories if needed, ports, protocols, and health paths. Then call `loopbox_validate_project_config`, explain any validation issues, and only call `loopbox_create_project` after user approval.
- Mutating Loopbox actions must go through MCP elicitation and wait for user approval.
- Ask before destructive changes, broad restarts, or replacing project configuration.
- Keep answers concise and include exact project/service names when giving commands or diagnoses.
- Do not assume a selected project. Treat the chat as global unless the user names a project or service."#
        .to_string()
}

fn thread_resume_params(thread_id: &str) -> Value {
    json!({
        "threadId": thread_id,
        "developerInstructions": loopbox_developer_instructions()
    })
}

fn parse_loopbox_mcp_tools(result: &Value) -> Vec<String> {
    let Some(servers) = result.get("data").and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut tools = Vec::new();
    for server in servers {
        if server.get("name").and_then(Value::as_str) != Some(LOOPBOX_MCP_SERVER_NAME) {
            continue;
        }
        let Some(tool_inventory) = server.get("tools") else {
            continue;
        };

        if let Some(tool_map) = tool_inventory.as_object() {
            for (key, tool) in tool_map {
                let name = tool
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(key.as_str())
                    .trim();
                if !name.is_empty() {
                    tools.push(name.to_string());
                }
            }
        } else if let Some(tool_list) = tool_inventory.as_array() {
            for tool in tool_list {
                if let Some(name) = tool.get("name").and_then(Value::as_str) {
                    let name = name.trim();
                    if !name.is_empty() {
                        tools.push(name.to_string());
                    }
                }
            }
        }
    }

    tools.sort();
    tools.dedup();
    tools
}

fn missing_loopbox_creation_tools(tools: &[String]) -> Vec<String> {
    REQUIRED_LOOPBOX_CREATION_TOOLS
        .iter()
        .filter(|required| !tools.iter().any(|tool| tool == **required))
        .map(|tool| (*tool).to_string())
        .collect()
}

fn codex_binary(config: &LoopboxConfig) -> String {
    config
        .global
        .codex_agents
        .codex_binary
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("codex")
        .to_string()
}

fn codex_sandbox_wire_value(value: &str) -> Option<&'static str> {
    match value.trim() {
        "" | "codexDefault" | "codex-default" => None,
        "readOnly" | "read-only" => Some("read-only"),
        "workspaceWrite" | "workspace-write" => Some("workspace-write"),
        "dangerFullAccess" | "danger-full-access" => Some("danger-full-access"),
        _ => None,
    }
}

fn should_keep_stderr_line(line: &str) -> bool {
    let line = line.trim();
    if line.is_empty() {
        return false;
    }

    let lower = line.to_ascii_lowercase();
    if lower.contains("app_server.request")
        || lower.contains("codex_app_server::app_server_tracing")
        || lower.contains("codex_core_plugins::manifest")
        || lower.contains("codex_core_skills::loader")
    {
        return false;
    }

    lower.starts_with("error")
        || lower.contains(" error")
        || lower.contains("error:")
        || lower.contains("failed")
        || lower.contains("fatal")
        || lower.contains("panic")
}

fn notification_error_message(method: &str, params: &Value) -> Option<String> {
    let message = match method {
        "error" => params.pointer("/error/message").and_then(Value::as_str),
        "turn/completed" => params
            .pointer("/turn/error/message")
            .and_then(Value::as_str),
        _ => None,
    }?;

    let details = params
        .pointer("/turn/error/additionalDetails")
        .or_else(|| params.pointer("/error/additionalDetails"))
        .and_then(Value::as_str)
        .filter(|details| !details.trim().is_empty());

    Some(match details {
        Some(details) => format!("{message}\n{details}"),
        None => message.to_string(),
    })
}

fn notification_item_type(params: &Value) -> Option<&str> {
    params.pointer("/item/type").and_then(Value::as_str)
}

fn strip_ansi(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && matches!(chars.peek(), Some('[')) {
            chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
            continue;
        }
        output.push(ch);
    }
    output
}

fn current_exe_path() -> Result<String, String> {
    std::env::current_exe()
        .map_err(|err| format!("Failed to resolve current Loopbox executable: {err}"))
        .map(|path| path.display().to_string())
}

fn toml_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| format!("\"{}\"", value.replace('"', "\\\"")))
}

fn session_lock() -> &'static Mutex<Option<Arc<CodexAppInner>>> {
    CODEX_SESSION.get_or_init(|| Mutex::new(None))
}

fn prefilled_prompt_lock() -> &'static Mutex<Option<String>> {
    PREFILLED_PROMPT.get_or_init(|| Mutex::new(None))
}

fn take_prefilled_prompt() -> Option<String> {
    prefilled_prompt_lock()
        .lock()
        .ok()
        .and_then(|mut slot| slot.take())
}

fn active_session() -> Result<Arc<CodexAppInner>, String> {
    session_lock()
        .lock()
        .map_err(|_| "Codex session lock poisoned.".to_string())?
        .clone()
        .ok_or_else(|| "Codex app-server is not running.".to_string())
}

fn push_limited(queue: &mut VecDeque<String>, value: String, limit: usize) {
    queue.push_back(value);
    while queue.len() > limit {
        queue.pop_front();
    }
}

#[allow(dead_code)]
fn unix_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_values_match_app_server_wire_names() {
        assert_eq!(
            codex_sandbox_wire_value("workspaceWrite"),
            Some("workspace-write")
        );
        assert_eq!(
            codex_sandbox_wire_value("workspace-write"),
            Some("workspace-write")
        );
        assert_eq!(codex_sandbox_wire_value("readOnly"), Some("read-only"));
        assert_eq!(
            codex_sandbox_wire_value("dangerFullAccess"),
            Some("danger-full-access")
        );
        assert_eq!(codex_sandbox_wire_value("codexDefault"), None);
        assert_eq!(codex_sandbox_wire_value("unknown"), None);
    }

    #[test]
    fn stderr_filter_strips_ansi_and_drops_tracing_noise() {
        let stripped = strip_ansi("\u{1b}[32mINFO\u{1b}[0m app_server.request exit");
        assert_eq!(stripped, "INFO app_server.request exit");
        assert!(!should_keep_stderr_line(&stripped));
        assert!(!should_keep_stderr_line(
            "WARN codex_app_server::app_server_tracing: enter"
        ));
        assert!(!should_keep_stderr_line(
            "2026-05-05T17:05:06Z WARN codex_core_plugins::manifest: ignoring interface.defaultPrompt"
        ));
        assert!(!should_keep_stderr_line(
            "2026-05-05T17:05:08Z WARN codex_core_skills::loader: ignoring interface.icon_small"
        ));
        assert!(should_keep_stderr_line(
            "WARN loopbox mcp server failed to start"
        ));
    }

    #[test]
    fn notification_errors_are_extracted_for_failed_turns() {
        let params = json!({
            "turn": {
                "status": "failed",
                "error": {
                    "message": "MCP server unavailable",
                    "additionalDetails": "loopbox failed to initialize"
                }
            }
        });

        assert_eq!(
            notification_error_message("turn/completed", &params),
            Some("MCP server unavailable\nloopbox failed to initialize".to_string())
        );
    }

    #[test]
    fn mcp_status_records_loopbox_tools_and_missing_creation_tools() {
        let status = json!({
            "data": [
                {
                    "name": "other",
                    "tools": {
                        "loopbox_create_project": {
                            "name": "loopbox_create_project",
                            "inputSchema": { "type": "object" }
                        }
                    }
                },
                {
                    "name": "loopbox",
                    "tools": {
                        "loopbox_runtime": {
                            "name": "loopbox_runtime",
                            "inputSchema": { "type": "object" }
                        },
                        "loopbox_validate_project_config": {
                            "name": "loopbox_validate_project_config",
                            "inputSchema": { "type": "object" }
                        }
                    }
                }
            ],
            "nextCursor": null
        });

        let tools = parse_loopbox_mcp_tools(&status);
        assert_eq!(
            tools,
            vec![
                "loopbox_runtime".to_string(),
                "loopbox_validate_project_config".to_string()
            ]
        );
        assert_eq!(
            missing_loopbox_creation_tools(&tools),
            vec!["loopbox_create_project".to_string()]
        );
    }

    #[test]
    fn mcp_status_detects_ready_creation_tools() {
        let status = json!({
            "data": [{
                "name": "loopbox",
                "tools": {
                    "validate": { "name": "loopbox_validate_project_config" },
                    "create": { "name": "loopbox_create_project" },
                    "update": { "name": "loopbox_update_project" }
                }
            }]
        });

        let tools = parse_loopbox_mcp_tools(&status);
        assert!(tools.contains(&"loopbox_validate_project_config".to_string()));
        assert!(tools.contains(&"loopbox_create_project".to_string()));
        assert!(missing_loopbox_creation_tools(&tools).is_empty());
    }

    #[test]
    fn thread_resume_params_include_current_developer_instructions() {
        let params = thread_resume_params("thread-123");

        assert_eq!(params.get("threadId"), Some(&json!("thread-123")));
        let instructions = params
            .get("developerInstructions")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(instructions.contains("sandbox"));
        assert!(instructions.contains("project"));
        assert!(instructions.contains("loopbox_validate_project_config"));
        assert!(instructions.contains("loopbox_create_project"));
    }
}
