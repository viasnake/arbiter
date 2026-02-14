use arbiter_config::Config;
use arbiter_contracts::{
    contracts_manifest_v1, ActionEnvelope, ActionResult, ActionType, ApprovalEvent,
    ApprovalPolicySummary, ContractsMetadata, ErrorBody, ErrorResponse, EventEnvelope,
    GovernanceView, PlanApproval, PlanDecision, PlanEnvelope, API_VERSION,
};
use arbiter_kernel::{
    jcs_sha256_hex, parse_rfc3339, pick_action_type, pick_risk, plan_params, stable_action_id,
    stable_plan_id,
};
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

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
    let state = AppState::new(cfg)?;
    Ok(Router::new()
        .route("/v1/healthz", get(healthz))
        .route("/v1/contracts", get(contracts))
        .route("/v1/events", post(events))
        .route("/v1/approval-events", post(approval_events))
        .route("/v1/action-results", post(action_results))
        .with_state(state))
}

#[derive(Clone)]
struct AppState {
    cfg: Config,
    store: Arc<Mutex<StoreBackend>>,
    contracts_metadata: Arc<ContractsMetadata>,
}

impl AppState {
    fn new(cfg: Config) -> Result<Self, String> {
        let store = match cfg.store.kind.as_str() {
            "memory" => StoreBackend::Memory(Box::new(MemoryStore {
                audit_path: cfg.audit.jsonl_path.clone(),
                audit_mirror_path: cfg.audit.immutable_mirror_path.clone(),
                ..Default::default()
            })),
            "sqlite" => {
                let sqlite_path = cfg.store.sqlite_path.clone().ok_or_else(|| {
                    "store.sqlite_path is required when store.kind=sqlite".to_string()
                })?;
                StoreBackend::Sqlite(SqliteStore::new(
                    &sqlite_path,
                    &cfg.audit.jsonl_path,
                    cfg.audit.immutable_mirror_path.clone(),
                )?)
            }
            _ => {
                return Err(format!(
                    "config.invalid_store_kind: unsupported store.kind `{}`",
                    cfg.store.kind
                ));
            }
        };

        let contracts_metadata = build_contracts_metadata(&cfg);
        Ok(Self {
            cfg,
            store: Arc::new(Mutex::new(store)),
            contracts_metadata: Arc::new(contracts_metadata),
        })
    }

    async fn process_event(&self, event: EventEnvelope) -> Result<PlanEnvelope, ApiFailure> {
        if parse_rfc3339(&event.occurred_at).is_none() {
            return Err(ApiFailure::bad_request(
                "request.schema_invalid",
                "occurred_at must be RFC3339",
            ));
        }

        let payload = serde_json::to_value(&event).map_err(|err| {
            ApiFailure::bad_request("request.schema_invalid", &format!("invalid payload: {err}"))
        })?;
        let incoming_hash = jcs_sha256_hex(&payload)
            .map_err(|err| ApiFailure::internal(&format!("failed to fingerprint event: {err}")))?;
        let id_key = event_store_key(&event.tenant_id, &event.event_id);

        if let Some(existing) = {
            let store = self.store.lock().await;
            store.get_event(&id_key)?
        } {
            if existing.payload_hash != incoming_hash {
                return Err(ApiFailure::conflict_payload_mismatch(
                    existing.payload_hash,
                    incoming_hash,
                ));
            }
            return Ok(existing.plan);
        }

        let plan = self.make_plan(&event)?;
        let plan_hash = jcs_sha256_hex(&serde_json::to_value(&plan).map_err(|err| {
            ApiFailure::internal(&format!("failed to serialize generated plan: {err}"))
        })?)
        .map_err(|err| ApiFailure::internal(&format!("failed to fingerprint plan: {err}")))?;

        {
            let mut store = self.store.lock().await;
            store.save_event(&id_key, &incoming_hash, &plan)?;
            store.append_audit(AuditRecord::new(
                "event_processed",
                &event.tenant_id,
                &event.event_id,
                Some(incoming_hash),
                Some(plan_hash),
            ))?;
        }

        Ok(plan)
    }

    fn make_plan(&self, event: &EventEnvelope) -> Result<PlanEnvelope, ApiFailure> {
        if let Some(raw_action_type) = event.labels.get("action_type") {
            if raw_action_type != "notify"
                && raw_action_type != "write_external"
                && raw_action_type != "start_job"
            {
                return Err(ApiFailure::bad_request(
                    "policy.action_type_not_allowed",
                    "action_type is not allowed by governance policy",
                ));
            }
        }
        let action_type = pick_action_type(event);
        let provider = event
            .labels
            .get("provider")
            .cloned()
            .unwrap_or_else(|| "generic".to_string());

        if !self.cfg.governance.allowed_providers.contains(&provider) {
            return Err(ApiFailure::bad_request(
                "policy.provider_not_allowed",
                "provider is not allowed by governance policy",
            ));
        }

        let requires_approval = match action_type {
            ActionType::Notify => self.cfg.policy.require_approval_for_notify,
            ActionType::WriteExternal => self.cfg.policy.require_approval_for_write_external,
            ActionType::StartJob => self.cfg.policy.require_approval_for_start_job,
        };

        let plan_id = stable_plan_id(&event.tenant_id, &event.event_id);
        let action_id = stable_action_id(&plan_id, action_type.clone());
        let operation = event
            .labels
            .get("operation")
            .cloned()
            .unwrap_or_else(|| "perform".to_string());

        let action = ActionEnvelope {
            action_id: action_id.clone(),
            action_type,
            provider,
            operation,
            params: plan_params(event),
            risk: pick_risk(event),
            requires_approval,
            idempotency_key: format!("{}:{}:{}", event.tenant_id, event.event_id, action_id),
        };

        let approval = if requires_approval {
            Some(PlanApproval {
                required: true,
                approval_id: Some(format!("apr_{}_{}", event.tenant_id, event.event_id)),
            })
        } else {
            Some(PlanApproval {
                required: false,
                approval_id: None,
            })
        };

        Ok(PlanEnvelope {
            plan_id,
            tenant_id: event.tenant_id.clone(),
            event_id: event.event_id.clone(),
            actions: vec![action],
            approval,
            decision: PlanDecision {
                policy_version: self.cfg.policy.version.clone(),
                evaluation_time: event.occurred_at.clone(),
                notes: None,
            },
        })
    }

    async fn process_approval_event(&self, input: ApprovalEvent) -> Result<(), ApiFailure> {
        if parse_rfc3339(&input.decided_at).is_none() {
            return Err(ApiFailure::bad_request(
                "request.schema_invalid",
                "decided_at must be RFC3339",
            ));
        }

        let payload_hash = jcs_sha256_hex(&serde_json::to_value(&input).map_err(|err| {
            ApiFailure::internal(&format!("failed to serialize approval event: {err}"))
        })?)
        .map_err(|err| ApiFailure::internal(&format!("failed to fingerprint approval: {err}")))?;

        let mut store = self.store.lock().await;
        store.append_audit(AuditRecord::new(
            "approval_event_recorded",
            &input.tenant_id,
            &input.approval_id,
            Some(payload_hash),
            None,
        ))?;
        Ok(())
    }

    async fn process_action_result(&self, input: ActionResult) -> Result<(), ApiFailure> {
        if parse_rfc3339(&input.occurred_at).is_none() {
            return Err(ApiFailure::bad_request(
                "request.schema_invalid",
                "occurred_at must be RFC3339",
            ));
        }
        let payload_hash = jcs_sha256_hex(&serde_json::to_value(&input).map_err(|err| {
            ApiFailure::internal(&format!("failed to serialize action-result: {err}"))
        })?)
        .map_err(|err| {
            ApiFailure::internal(&format!("failed to fingerprint action-result: {err}"))
        })?;

        let key = action_result_store_key(&input.tenant_id, &input.plan_id, &input.action_id);
        let mut store = self.store.lock().await;
        if let Some(existing_hash) = store.get_action_result_hash(&key)? {
            if existing_hash != payload_hash {
                return Err(ApiFailure::conflict_payload_mismatch(
                    existing_hash,
                    payload_hash,
                ));
            }
            return Ok(());
        }

        store.save_action_result_hash(&key, &payload_hash)?;
        store.append_audit(AuditRecord::new(
            "action_result_recorded",
            &input.tenant_id,
            &input.action_id,
            Some(payload_hash),
            None,
        ))?;
        Ok(())
    }
}

async fn healthz() -> (StatusCode, &'static str) {
    (StatusCode::OK, "ok")
}

async fn contracts(State(state): State<AppState>) -> Json<ContractsMetadata> {
    Json((*state.contracts_metadata).clone())
}

async fn events(
    State(state): State<AppState>,
    Json(input): Json<EventEnvelope>,
) -> Result<Json<PlanEnvelope>, (StatusCode, Json<ErrorResponse>)> {
    state
        .process_event(input)
        .await
        .map(Json)
        .map_err(into_error)
}

async fn approval_events(
    State(state): State<AppState>,
    Json(input): Json<ApprovalEvent>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .process_approval_event(input)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(into_error)
}

async fn action_results(
    State(state): State<AppState>,
    Json(input): Json<ActionResult>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .process_action_result(input)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(into_error)
}

#[derive(Default)]
struct MemoryStore {
    events: HashMap<String, StoredEvent>,
    action_result_hashes: HashMap<String, String>,
    audit_last_hash: Option<String>,
    audit_path: String,
    audit_mirror_path: Option<String>,
}

struct StoredEvent {
    payload_hash: String,
    plan: PlanEnvelope,
}

enum StoreBackend {
    Memory(Box<MemoryStore>),
    Sqlite(SqliteStore),
}

struct SqliteStore {
    conn: Connection,
    audit_path: String,
    audit_mirror_path: Option<String>,
}

impl SqliteStore {
    fn new(
        path: &str,
        audit_path: &str,
        audit_mirror_path: Option<String>,
    ) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|err| err.to_string())?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS events (
                key TEXT PRIMARY KEY,
                payload_hash TEXT NOT NULL,
                plan_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS action_results (
                key TEXT PRIMARY KEY,
                payload_hash TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS audit_records (
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                record_hash TEXT NOT NULL,
                record_json TEXT NOT NULL
            );
            ",
        )
        .map_err(|err| err.to_string())?;

        Ok(Self {
            conn,
            audit_path: audit_path.to_string(),
            audit_mirror_path,
        })
    }
}

impl StoreBackend {
    fn get_event(&self, key: &str) -> Result<Option<StoredEvent>, ApiFailure> {
        match self {
            StoreBackend::Memory(store) => Ok(store.events.get(key).map(|v| StoredEvent {
                payload_hash: v.payload_hash.clone(),
                plan: v.plan.clone(),
            })),
            StoreBackend::Sqlite(store) => {
                let row: Option<(String, String)> = store
                    .conn
                    .query_row(
                        "SELECT payload_hash, plan_json FROM events WHERE key=?1",
                        params![key],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()
                    .map_err(|err| ApiFailure::internal(&err.to_string()))?;

                row.map(|(payload_hash, plan_json)| {
                    serde_json::from_str::<PlanEnvelope>(&plan_json)
                        .map(|plan| StoredEvent { payload_hash, plan })
                        .map_err(|err| {
                            ApiFailure::internal(&format!(
                                "failed to deserialize stored plan: {err}"
                            ))
                        })
                })
                .transpose()
            }
        }
    }

    fn save_event(
        &mut self,
        key: &str,
        payload_hash: &str,
        plan: &PlanEnvelope,
    ) -> Result<(), ApiFailure> {
        match self {
            StoreBackend::Memory(store) => {
                store.events.insert(
                    key.to_string(),
                    StoredEvent {
                        payload_hash: payload_hash.to_string(),
                        plan: plan.clone(),
                    },
                );
                Ok(())
            }
            StoreBackend::Sqlite(store) => {
                let plan_json = serde_json::to_string(plan).map_err(|err| {
                    ApiFailure::internal(&format!("failed to encode plan: {err}"))
                })?;
                store
                    .conn
                    .execute(
                        "INSERT INTO events(key, payload_hash, plan_json) VALUES(?1, ?2, ?3)",
                        params![key, payload_hash, plan_json],
                    )
                    .map_err(|err| ApiFailure::internal(&err.to_string()))?;
                Ok(())
            }
        }
    }

    fn get_action_result_hash(&self, key: &str) -> Result<Option<String>, ApiFailure> {
        match self {
            StoreBackend::Memory(store) => Ok(store.action_result_hashes.get(key).cloned()),
            StoreBackend::Sqlite(store) => store
                .conn
                .query_row(
                    "SELECT payload_hash FROM action_results WHERE key=?1",
                    params![key],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|err| ApiFailure::internal(&err.to_string())),
        }
    }

    fn save_action_result_hash(&mut self, key: &str, payload_hash: &str) -> Result<(), ApiFailure> {
        match self {
            StoreBackend::Memory(store) => {
                store
                    .action_result_hashes
                    .insert(key.to_string(), payload_hash.to_string());
                Ok(())
            }
            StoreBackend::Sqlite(store) => {
                store
                    .conn
                    .execute(
                        "INSERT INTO action_results(key, payload_hash) VALUES(?1, ?2)",
                        params![key, payload_hash],
                    )
                    .map_err(|err| ApiFailure::internal(&err.to_string()))?;
                Ok(())
            }
        }
    }

    fn append_audit(&mut self, record: AuditRecord) -> Result<(), ApiFailure> {
        match self {
            StoreBackend::Memory(store) => {
                let prev_hash = store.audit_last_hash.clone().unwrap_or_default();
                let entry = record.into_entry(prev_hash)?;
                append_jsonl_line(&store.audit_path, &entry)?;
                if let Some(path) = store.audit_mirror_path.as_deref() {
                    append_jsonl_line(path, &entry)?;
                }
                store.audit_last_hash = Some(entry.record_hash.clone());
                Ok(())
            }
            StoreBackend::Sqlite(store) => {
                let prev_hash: Option<String> = store
                    .conn
                    .query_row(
                        "SELECT record_hash FROM audit_records ORDER BY seq DESC LIMIT 1",
                        [],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(|err| ApiFailure::internal(&err.to_string()))?;
                let entry = record.into_entry(prev_hash.unwrap_or_default())?;
                let record_json = serde_json::to_string(&entry)
                    .map_err(|err| ApiFailure::internal(&format!("audit encode failed: {err}")))?;

                store
                    .conn
                    .execute(
                        "INSERT INTO audit_records(record_hash, record_json) VALUES(?1, ?2)",
                        params![entry.record_hash, record_json],
                    )
                    .map_err(|err| ApiFailure::internal(&err.to_string()))?;
                append_jsonl_line(&store.audit_path, &entry)?;
                if let Some(path) = store.audit_mirror_path.as_deref() {
                    append_jsonl_line(path, &entry)?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Serialize)]
struct AuditEntry {
    recorded_at: String,
    kind: String,
    tenant_id: String,
    reference_id: String,
    input_fingerprint: Option<String>,
    output_fingerprint: Option<String>,
    prev_hash: String,
    record_hash: String,
}

struct AuditRecord {
    kind: String,
    tenant_id: String,
    reference_id: String,
    input_fingerprint: Option<String>,
    output_fingerprint: Option<String>,
}

impl AuditRecord {
    fn new(
        kind: &str,
        tenant_id: &str,
        reference_id: &str,
        input_fingerprint: Option<String>,
        output_fingerprint: Option<String>,
    ) -> Self {
        Self {
            kind: kind.to_string(),
            tenant_id: tenant_id.to_string(),
            reference_id: reference_id.to_string(),
            input_fingerprint,
            output_fingerprint,
        }
    }

    fn into_entry(self, prev_hash: String) -> Result<AuditEntry, ApiFailure> {
        let recorded_at = Utc::now().to_rfc3339();
        let seed = json!({
            "recorded_at": recorded_at,
            "kind": self.kind,
            "tenant_id": self.tenant_id,
            "reference_id": self.reference_id,
            "input_fingerprint": self.input_fingerprint,
            "output_fingerprint": self.output_fingerprint,
            "prev_hash": prev_hash,
        });
        let record_hash = jcs_sha256_hex(&seed)
            .map_err(|err| ApiFailure::internal(&format!("audit hash failed: {err}")))?;

        Ok(AuditEntry {
            recorded_at: seed["recorded_at"].as_str().unwrap_or_default().to_string(),
            kind: seed["kind"].as_str().unwrap_or_default().to_string(),
            tenant_id: seed["tenant_id"].as_str().unwrap_or_default().to_string(),
            reference_id: seed["reference_id"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            input_fingerprint: seed["input_fingerprint"].as_str().map(|v| v.to_string()),
            output_fingerprint: seed["output_fingerprint"].as_str().map(|v| v.to_string()),
            prev_hash: seed["prev_hash"].as_str().unwrap_or_default().to_string(),
            record_hash,
        })
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

fn event_store_key(tenant_id: &str, event_id: &str) -> String {
    format!("{tenant_id}:{event_id}")
}

fn action_result_store_key(tenant_id: &str, plan_id: &str, action_id: &str) -> String {
    format!("{tenant_id}:{plan_id}:{action_id}")
}

fn build_contracts_metadata(cfg: &Config) -> ContractsMetadata {
    let manifest = contracts_manifest_v1();
    let schemas = manifest
        .schemas
        .iter()
        .map(|v| (v.path.to_string(), v.sha256.to_string()))
        .collect::<BTreeMap<_, _>>();
    let mut defaults = BTreeMap::new();
    defaults.insert("notify".to_string(), cfg.policy.require_approval_for_notify);
    defaults.insert(
        "write_external".to_string(),
        cfg.policy.require_approval_for_write_external,
    );
    defaults.insert(
        "start_job".to_string(),
        cfg.policy.require_approval_for_start_job,
    );
    let mut required_for_types = Vec::new();
    if cfg.policy.require_approval_for_notify {
        required_for_types.push(ActionType::Notify);
    }
    if cfg.policy.require_approval_for_write_external {
        required_for_types.push(ActionType::WriteExternal);
    }
    if cfg.policy.require_approval_for_start_job {
        required_for_types.push(ActionType::StartJob);
    }

    ContractsMetadata {
        api_version: API_VERSION.to_string(),
        openapi_sha256: manifest.openapi_sha256.to_string(),
        contracts_set_sha256: manifest.contracts_set_sha256.to_string(),
        generated_at: manifest.generated_at.to_string(),
        schemas,
        governance: GovernanceView {
            allowed_action_types: vec![
                ActionType::Notify,
                ActionType::WriteExternal,
                ActionType::StartJob,
            ],
            allowed_providers: cfg.governance.allowed_providers.clone(),
            approval_policy: ApprovalPolicySummary {
                required_for_types,
                defaults,
            },
            max_payload_hints: Some(BTreeMap::from([
                ("event_summary_max_bytes".to_string(), 4096),
                ("action_params_max_bytes".to_string(), 16384),
            ])),
            error_codes: Some("docs/spec/errors.md".to_string()),
        },
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

    fn internal(message: &str) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal.error".to_string(),
            message: message.to_string(),
            details: None,
        }
    }

    fn conflict_payload_mismatch(existing_hash: String, incoming_hash: String) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: "conflict.payload_mismatch".to_string(),
            message: "idempotency key is reused with different payload".to_string(),
            details: Some(json!({
                "existing_hash": existing_hash,
                "incoming_hash": incoming_hash,
            })),
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
