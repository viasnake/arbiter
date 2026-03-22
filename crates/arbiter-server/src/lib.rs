use arbiter_config::Config;
use arbiter_contracts::{
    contracts_manifest_v1, ApprovalRequest, ContractsMetadata, Decision, DecisionEffect, ErrorBody,
    ErrorResponse, ExecutionPermit, OperationRequest, Run, RunEnvelope, RunStatus, Step,
    StepIntent, StepStatus, StepType, API_VERSION,
};
use arbiter_kernel::jcs_sha256_hex;
use axum::extract::{Path as AxPath, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{Duration, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

pub async fn serve(cfg: Config) -> Result<(), String> {
    let addr: SocketAddr = cfg
        .server
        .listen_addr
        .parse()
        .map_err(|err| format!("invalid listen_addr: {err}"))?;
    let app = build_app(cfg).await?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|err| format!("bind failed: {err}"))?;
    axum::serve(listener, app)
        .await
        .map_err(|err| format!("serve failed: {err}"))
}

pub async fn build_app(cfg: Config) -> Result<Router, String> {
    let state = AppState::new(cfg);
    Ok(Router::new()
        .route("/v1/healthz", get(healthz))
        .route("/v1/contracts", get(contracts))
        .route("/v2/operation-requests", post(create_run))
        .route("/v2/runs/{run_id}", get(get_run))
        .route("/v2/runs/{run_id}/step-intents", post(submit_step_intent))
        .route("/v2/approvals/{approval_id}/grant", post(grant_approval))
        .with_state(state))
}

#[derive(Clone)]
struct AppState {
    store: Arc<Mutex<MemoryStore>>,
    contracts_metadata: Arc<ContractsMetadata>,
}

impl AppState {
    fn new(cfg: Config) -> Self {
        let contracts_metadata = build_contracts_metadata();
        Self {
            store: Arc::new(Mutex::new(MemoryStore {
                runs: HashMap::new(),
                approvals: HashMap::new(),
                audit_last_hash: None,
                audit_path: cfg.audit.jsonl_path,
                audit_mirror_path: cfg.audit.immutable_mirror_path,
            })),
            contracts_metadata: Arc::new(contracts_metadata),
        }
    }
}

#[derive(Default)]
struct MemoryStore {
    runs: HashMap<String, RunEnvelope>,
    approvals: HashMap<String, String>,
    audit_last_hash: Option<String>,
    audit_path: String,
    audit_mirror_path: Option<String>,
}

async fn healthz() -> (StatusCode, &'static str) {
    (StatusCode::OK, "ok")
}

async fn contracts(State(state): State<AppState>) -> Json<ContractsMetadata> {
    Json((*state.contracts_metadata).clone())
}

async fn create_run(
    State(state): State<AppState>,
    Json(input): Json<OperationRequest>,
) -> Result<(StatusCode, Json<RunEnvelope>), (StatusCode, Json<ErrorResponse>)> {
    let now = Utc::now().to_rfc3339();
    let run_id = format!("run_{}", Uuid::new_v4().simple());
    let run = Run {
        run_id: run_id.clone(),
        request_id: input.request_id.clone(),
        agent_id: input.target_agent,
        executor_id: "executor-default".to_string(),
        status: RunStatus::Running,
        created_at: now.clone(),
        started_at: Some(now),
        completed_at: None,
        policy_snapshot: json!({"mode": "policy_pipeline"}),
        budget_snapshot: json!({}),
        lease_owner: "executor-default".to_string(),
    };
    let envelope = RunEnvelope { run, steps: vec![] };

    let mut store = state.store.lock().await;
    store.runs.insert(run_id.clone(), envelope.clone());
    store
        .append_audit(AuditRecord::new(
            "run_created",
            &run_id,
            json!({"request_id": input.request_id, "requester": input.requester}),
        ))
        .map_err(into_error)?;

    Ok((StatusCode::CREATED, Json(envelope)))
}

async fn get_run(
    State(state): State<AppState>,
    AxPath(run_id): AxPath<String>,
) -> Result<Json<RunEnvelope>, (StatusCode, Json<ErrorResponse>)> {
    let store = state.store.lock().await;
    let run = store
        .runs
        .get(&run_id)
        .cloned()
        .ok_or_else(|| ApiFailure::not_found("run.not_found", "run not found"))
        .map_err(into_error)?;
    Ok(Json(run))
}

async fn submit_step_intent(
    State(state): State<AppState>,
    AxPath(run_id): AxPath<String>,
    Json(intent): Json<StepIntent>,
) -> Result<Json<Step>, (StatusCode, Json<ErrorResponse>)> {
    let mut store = state.store.lock().await;
    let mut run = store
        .runs
        .remove(&run_id)
        .ok_or_else(|| ApiFailure::not_found("run.not_found", "run not found"))
        .map_err(into_error)?;

    if run.run.status == RunStatus::Cancelled
        || run.run.status == RunStatus::Completed
        || run.run.status == RunStatus::TimedOut
    {
        return Err(into_error(ApiFailure::bad_request(
            "run.invalid_state",
            "cannot accept step-intent in terminal run state",
        )));
    }

    let needs_approval = intent.step_type == StepType::ToolCall
        && matches!(intent.risk_level.as_str(), "write" | "external" | "high");

    let (decision, permit, approval_request, next_run_status, step_status) = if needs_approval {
        let approval_id = format!("apr_{}", Uuid::new_v4().simple());
        let expires_at = (Utc::now() + Duration::minutes(30)).to_rfc3339();
        let request = ApprovalRequest {
            approval_id: approval_id.clone(),
            run_id: run_id.clone(),
            step_id: format!("step_{}", Uuid::new_v4().simple()),
            requested_action: intent.proposed_action.clone(),
            approver_set: vec!["human-approver".to_string()],
            status: "requested".to_string(),
            reason: "write/external operation requires approval".to_string(),
            expires_at,
        };

        store.approvals.insert(approval_id, run_id.clone());

        (
            Decision {
                decision_id: format!("dec_{}", Uuid::new_v4().simple()),
                effect: DecisionEffect::RequireApproval,
                reason: "approval required for write/external step".to_string(),
                applied_policies: vec!["approval.require_for_write".to_string()],
                constraints: json!({}),
                required_approvers: vec!["human-approver".to_string()],
                executor_scope: Some("executor-default".to_string()),
                expires_at: None,
            },
            None,
            Some(request),
            RunStatus::WaitingForApproval,
            StepStatus::WaitingForApproval,
        )
    } else {
        let issued = Utc::now();
        let permit = ExecutionPermit {
            permit_id: format!("permit_{}", Uuid::new_v4().simple()),
            run_id: run_id.clone(),
            step_id: format!("step_{}", Uuid::new_v4().simple()),
            executor_id: run.run.executor_id.clone(),
            allowed_action: intent.proposed_action.clone(),
            constraints: json!({"mode": "normal"}),
            issued_at: issued.to_rfc3339(),
            expires_at: (issued + Duration::minutes(5)).to_rfc3339(),
            token: Uuid::new_v4().to_string(),
        };

        (
            Decision {
                decision_id: format!("dec_{}", Uuid::new_v4().simple()),
                effect: DecisionEffect::Allow,
                reason: "policy allows step".to_string(),
                applied_policies: vec!["default_allow".to_string()],
                constraints: json!({}),
                required_approvers: vec![],
                executor_scope: Some("executor-default".to_string()),
                expires_at: Some(permit.expires_at.clone()),
            },
            Some(permit),
            None,
            RunStatus::Running,
            StepStatus::Permitted,
        )
    };

    run.run.status = next_run_status;
    let step = Step {
        step_id: format!("step_{}", Uuid::new_v4().simple()),
        run_id: run_id.clone(),
        step_type: intent.step_type.clone(),
        status: step_status,
        intent,
        decision,
        permit,
        approval_request,
        started_at: Utc::now().to_rfc3339(),
        completed_at: None,
    };
    run.steps.push(step.clone());
    store.runs.insert(run_id.clone(), run);

    store
        .append_audit(AuditRecord::new(
            "step_intent_received",
            &run_id,
            json!({"step_id": step.step_id, "decision": step.decision.effect}),
        ))
        .map_err(into_error)?;

    Ok(Json(step))
}

async fn grant_approval(
    State(state): State<AppState>,
    AxPath(approval_id): AxPath<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let mut store = state.store.lock().await;
    let run_id = store
        .approvals
        .get(&approval_id)
        .cloned()
        .ok_or_else(|| ApiFailure::not_found("approval.not_found", "approval not found"))
        .map_err(into_error)?;

    let run = store
        .runs
        .get_mut(&run_id)
        .ok_or_else(|| ApiFailure::not_found("run.not_found", "run not found"))
        .map_err(into_error)?;
    run.run.status = RunStatus::Running;

    for step in run.steps.iter_mut().rev() {
        if let Some(req) = step.approval_request.as_mut() {
            if req.approval_id == approval_id {
                req.status = "approved".to_string();
                step.status = StepStatus::Permitted;
                let issued = Utc::now();
                step.permit = Some(ExecutionPermit {
                    permit_id: format!("permit_{}", Uuid::new_v4().simple()),
                    run_id: run_id.clone(),
                    step_id: step.step_id.clone(),
                    executor_id: run.run.executor_id.clone(),
                    allowed_action: step.intent.proposed_action.clone(),
                    constraints: json!({"approved": true}),
                    issued_at: issued.to_rfc3339(),
                    expires_at: (issued + Duration::minutes(5)).to_rfc3339(),
                    token: Uuid::new_v4().to_string(),
                });
                break;
            }
        }
    }

    store
        .append_audit(AuditRecord::new(
            "approval_granted",
            &run_id,
            json!({"approval_id": approval_id}),
        ))
        .map_err(into_error)?;

    Ok(StatusCode::NO_CONTENT)
}

fn build_contracts_metadata() -> ContractsMetadata {
    let manifest = contracts_manifest_v1();
    let schemas = manifest
        .schemas
        .iter()
        .map(|v| (v.path.to_string(), v.sha256.to_string()))
        .collect::<BTreeMap<_, _>>();

    ContractsMetadata {
        api_version: API_VERSION.to_string(),
        openapi_sha256: manifest.openapi_sha256.to_string(),
        contracts_set_sha256: manifest.contracts_set_sha256.to_string(),
        generated_at: manifest.generated_at.to_string(),
        schemas,
    }
}

#[derive(Debug)]
struct ApiFailure {
    status: StatusCode,
    code: String,
    message: String,
    details: Option<Value>,
}

impl ApiFailure {
    fn bad_request(code: &str, message: &str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: code.to_string(),
            message: message.to_string(),
            details: None,
        }
    }

    fn not_found(code: &str, message: &str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: code.to_string(),
            message: message.to_string(),
            details: None,
        }
    }

    fn internal(message: &str) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal.error".to_string(),
            message: message.to_string(),
            details: None,
        }
    }
}

fn into_error(err: ApiFailure) -> (StatusCode, Json<ErrorResponse>) {
    (
        err.status,
        Json(ErrorResponse {
            error: ErrorBody {
                code: err.code,
                message: err.message,
                details: err.details,
            },
        }),
    )
}

#[derive(Serialize)]
struct AuditEntry {
    recorded_at: String,
    event_type: String,
    run_id: String,
    payload: Value,
    prev_hash: String,
    record_hash: String,
}

struct AuditRecord {
    event_type: String,
    run_id: String,
    payload: Value,
}

impl AuditRecord {
    fn new(event_type: &str, run_id: &str, payload: Value) -> Self {
        Self {
            event_type: event_type.to_string(),
            run_id: run_id.to_string(),
            payload,
        }
    }

    fn into_entry(self, prev_hash: String) -> Result<AuditEntry, ApiFailure> {
        let recorded_at = Utc::now().to_rfc3339();
        let seed = json!({
            "recorded_at": recorded_at,
            "event_type": self.event_type,
            "run_id": self.run_id,
            "payload": self.payload,
            "prev_hash": prev_hash,
        });
        let record_hash = jcs_sha256_hex(&seed)
            .map_err(|err| ApiFailure::internal(&format!("audit hash failed: {err}")))?;

        Ok(AuditEntry {
            recorded_at: seed["recorded_at"].as_str().unwrap_or_default().to_string(),
            event_type: seed["event_type"].as_str().unwrap_or_default().to_string(),
            run_id: seed["run_id"].as_str().unwrap_or_default().to_string(),
            payload: seed["payload"].clone(),
            prev_hash: seed["prev_hash"].as_str().unwrap_or_default().to_string(),
            record_hash,
        })
    }
}

impl MemoryStore {
    fn append_audit(&mut self, record: AuditRecord) -> Result<(), ApiFailure> {
        let prev_hash = self.audit_last_hash.clone().unwrap_or_default();
        let entry = record.into_entry(prev_hash)?;
        append_jsonl_line(&self.audit_path, &entry)?;
        if let Some(path) = self.audit_mirror_path.as_deref() {
            append_jsonl_line(path, &entry)?;
        }
        self.audit_last_hash = Some(entry.record_hash.clone());
        Ok(())
    }
}

fn append_jsonl_line(path: &str, entry: &AuditEntry) -> Result<(), ApiFailure> {
    let file_path = Path::new(path);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file_path)
        .map_err(|err| ApiFailure::internal(&format!("failed to open audit file: {err}")))?;
    let line = serde_json::to_string(entry)
        .map_err(|err| ApiFailure::internal(&format!("failed to encode audit entry: {err}")))?;
    file.write_all(line.as_bytes())
        .map_err(|err| ApiFailure::internal(&format!("failed to write audit entry: {err}")))?;
    file.write_all(b"\n")
        .map_err(|err| ApiFailure::internal(&format!("failed to terminate audit entry: {err}")))?;
    Ok(())
}

pub fn verify_audit_chain(path: &str) -> Result<String, String> {
    verify_audit_chain_with_mirror(path, None)
}

pub fn verify_audit_chain_with_mirror(
    path: &str,
    mirror_path: Option<&str>,
) -> Result<String, String> {
    let main_lines = read_jsonl(path)?;
    let mut prev_hash = String::new();
    for (idx, line) in main_lines.iter().enumerate() {
        let value: Value = serde_json::from_str(line)
            .map_err(|err| format!("invalid json at line {}: {err}", idx + 1))?;
        let obj = value
            .as_object()
            .ok_or_else(|| format!("invalid record at line {}", idx + 1))?;
        let current_hash = obj
            .get("record_hash")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("record_hash missing at line {}", idx + 1))?;
        let current_prev = obj
            .get("prev_hash")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("prev_hash missing at line {}", idx + 1))?;
        if current_prev != prev_hash {
            return Err(format!(
                "hash chain mismatch at line {}: expected prev_hash {}, got {}",
                idx + 1,
                prev_hash,
                current_prev
            ));
        }

        let mut seed = obj.clone();
        seed.remove("record_hash");
        let recalculated = jcs_sha256_hex(&Value::Object(seed))
            .map_err(|err| format!("failed to hash record at line {}: {err}", idx + 1))?;
        if recalculated != current_hash {
            return Err(format!(
                "record hash mismatch at line {}: expected {}, got {}",
                idx + 1,
                current_hash,
                recalculated
            ));
        }
        prev_hash = current_hash.to_string();
    }

    if let Some(mirror) = mirror_path {
        let mirror_lines = read_jsonl(mirror)?;
        if main_lines != mirror_lines {
            return Err("mirror mismatch: audit and mirror contents differ".to_string());
        }
    }

    Ok(format!(
        "audit chain verified: {} records",
        main_lines.len()
    ))
}

fn read_jsonl(path: &str) -> Result<Vec<String>, String> {
    let text =
        std::fs::read_to_string(path).map_err(|err| format!("read failed for {path}: {err}"))?;
    Ok(text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.to_string())
        .collect())
}
