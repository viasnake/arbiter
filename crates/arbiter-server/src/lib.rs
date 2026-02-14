use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use arbiter_config::Config;
use arbiter_contracts::{
    contracts_manifest_v1, Action, ActionResult, ActionType, ApprovalEvent, AuthZDecision,
    AuthZReqData, AuthZRequest, AuthZResource, Event, GenerationResult, JobCancelRequest,
    JobStatusEvent, ResponsePlan, CONTRACT_VERSION,
};
use arbiter_kernel::{
    decide_intent, do_nothing_plan, evaluate_gate, minute_bucket, parse_event_ts,
    planner_probability, planner_seed, request_approval_plan, request_generation_plan, send_plan,
    start_agent_job_plan, GateConfig, GateDecision, Intent, PlannerConfig, RoomState,
};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use reqwest::Client;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use tokio::time::sleep;

pub async fn serve(cfg: Config) -> Result<(), String> {
    let addr: SocketAddr = cfg
        .server
        .listen_addr
        .parse()
        .map_err(|e| format!("invalid listen_addr: {e}"))?;

    let app = build_app(cfg).await?;

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("bind failed: {e}"))?;
    axum::serve(listener, app)
        .await
        .map_err(|e| format!("serve failed: {e}"))
}

pub async fn build_app(cfg: Config) -> Result<Router, String> {
    let state = AppState::new(cfg).await?;
    Ok(Router::new()
        .route("/v1/healthz", get(healthz))
        .route("/v1/events", post(events))
        .route("/v1/generations", post(generations))
        .route("/v1/job-events", post(job_events))
        .route("/v1/job-cancel", post(job_cancel))
        .route("/v1/approval-events", post(approval_events))
        .route("/v1/action-results", post(action_results))
        .route(
            "/v1/action-results/{tenant_id}/{plan_id}/{action_id}",
            get(action_result_state),
        )
        .route("/v1/contracts", get(contracts))
        .route("/v1/jobs/{tenant_id}/{job_id}", get(job_state))
        .route(
            "/v1/approvals/{tenant_id}/{approval_id}",
            get(approval_state),
        )
        .with_state(state))
}

#[derive(Clone)]
struct AppState {
    cfg: Config,
    store: Arc<Mutex<StoreBackend>>,
    audit: Arc<AuditJsonl>,
    authz: Arc<AuthzEngine>,
    contracts_metadata: Arc<Value>,
}

impl AppState {
    async fn new(cfg: Config) -> Result<Self, String> {
        let store = match cfg.store.kind.as_str() {
            "memory" => StoreBackend::Memory(Box::default()),
            "sqlite" => {
                let sqlite_path =
                    cfg.store.sqlite_path.clone().ok_or_else(|| {
                        "store.sqlite_path is required for sqlite store".to_string()
                    })?;
                StoreBackend::Sqlite(SqliteStore::new(&sqlite_path)?)
            }
            _ => {
                return Err(format!(
                    "config.invalid_store_kind: unsupported store.kind `{}` (expected memory|sqlite)",
                    cfg.store.kind
                ));
            }
        };
        Ok(Self {
            authz: Arc::new(AuthzEngine::new(&cfg)?),
            audit: Arc::new(
                AuditJsonl::new(
                    &cfg.audit.jsonl_path,
                    cfg.store.sqlite_path.as_deref(),
                    cfg.audit.immutable_mirror_path.as_deref(),
                )
                .await?,
            ),
            contracts_metadata: Arc::new(generate_contracts_metadata()?),
            store: Arc::new(Mutex::new(store)),
            cfg,
        })
    }

    async fn process_event(&self, event: Event) -> Result<ResponsePlan, String> {
        validate_event(&event)?;

        let key = event_key(&event.tenant_id, &event.event_id);
        let payload_json = canonical_json_string(
            &serde_json::to_value(&event).map_err(|e| format!("validation_error: {e}"))?,
        );
        let incoming_hash = hash_hex(payload_json.as_bytes());

        if let Some(existing) = {
            let store = self.store.lock().await;
            store.get_idempotency(&key)
        } {
            let maybe_existing_payload = {
                let store = self.store.lock().await;
                store.get_event_payload(&key)
            }
            .map_err(|e| format!("validation_error: {e}"))?;

            if let Some(existing_payload) = maybe_existing_payload {
                let existing_hash = if is_sha256_hex(&existing_payload) {
                    existing_payload
                } else {
                    hash_hex(existing_payload.as_bytes())
                };
                if existing_hash != incoming_hash {
                    return Err(format!(
                        "conflict.payload_mismatch: duplicate event_id has different payload (existing_hash={existing_hash}, incoming_hash={incoming_hash})"
                    ));
                }
            }

            self.audit
                .append(AuditRecord::new(
                    &event.tenant_id,
                    &event.event_id,
                    "process_event",
                    "idempotency_hit",
                    "idempotency_hit",
                    Some(existing.plan_id.clone()),
                ))
                .await
                .map_err(|e| format!("internal.audit_write_failed: {e}"))?;
            return Ok(existing);
        }

        parse_event_ts(&event.ts).ok_or_else(|| "invalid ts (RFC3339 required)".to_string())?;
        let server_now = Utc::now();

        let mut store = self.store.lock().await;
        let room_key = room_key(&event.tenant_id, &event.room_id);
        let tenant_bucket = minute_bucket(server_now);
        let room = store.get_room(&room_key);
        let tenant_count = store.get_tenant_rate_count(&event.tenant_id, tenant_bucket);

        let gate_cfg = GateConfig {
            cooldown_ms: self.cfg.gate.cooldown_ms,
            max_queue: self.cfg.gate.max_queue,
            tenant_rate_limit_per_min: self.cfg.gate.tenant_rate_limit_per_min,
        };
        if let GateDecision::Deny { reason_code } =
            evaluate_gate(&room, server_now, &gate_cfg, tenant_count)
        {
            let plan = do_nothing_plan(
                &event.tenant_id,
                &event.room_id,
                &event.event_id,
                reason_code,
            );
            store
                .save_idempotency(key.clone(), &plan)
                .map_err(|e| e.to_string())?;
            store
                .save_event_payload(&key, &incoming_hash)
                .map_err(|e| e.to_string())?;
            drop(store);

            self.audit
                .append(
                    AuditRecord::new(
                        &event.tenant_id,
                        &event.event_id,
                        "gate",
                        "deny",
                        reason_code,
                        Some(plan.plan_id.clone()),
                    )
                    .with_trace(DecisionTrace {
                        gate: Some(StageDecision {
                            result: "deny".to_string(),
                            reason_code: reason_code.to_string(),
                        }),
                        authz: None,
                        planner: None,
                    }),
                )
                .await
                .map_err(|e| format!("internal.audit_write_failed: {e}"))?;
            return Ok(plan);
        }
        drop(store);

        let authz = self.authz.authorize(&event).await;
        if !authz.allow {
            let plan = do_nothing_plan(
                &event.tenant_id,
                &event.room_id,
                &event.event_id,
                &authz.reason_code,
            );
            let mut store = self.store.lock().await;
            store
                .save_idempotency(key.clone(), &plan)
                .map_err(|e| e.to_string())?;
            store
                .save_event_payload(&key, &incoming_hash)
                .map_err(|e| e.to_string())?;
            drop(store);

            self.audit
                .append(
                    AuditRecord::new(
                        &event.tenant_id,
                        &event.event_id,
                        "authz",
                        "deny",
                        &authz.reason_code,
                        Some(plan.plan_id.clone()),
                    )
                    .with_trace(DecisionTrace {
                        gate: Some(StageDecision {
                            result: "allow".to_string(),
                            reason_code: "gate_allow".to_string(),
                        }),
                        authz: if self.cfg.audit.include_authz_decision {
                            Some(AuthzDecisionTrace {
                                result: "deny".to_string(),
                                reason_code: authz.reason_code.clone(),
                                policy_version: authz.policy_version.clone(),
                            })
                        } else {
                            None
                        },
                        planner: None,
                    }),
                )
                .await
                .map_err(|e| format!("internal.audit_write_failed: {e}"))?;
            return Ok(plan);
        }

        let planner_cfg = PlannerConfig {
            reply_policy: self.cfg.planner.reply_policy.clone(),
            reply_probability: self.cfg.planner.reply_probability,
        };
        let intent = decide_intent(&event, &planner_cfg);
        let planner_seed = planner_seed(&event.event_id);
        let sampled_probability = planner_probability(&event.event_id);

        let mut plan = match intent {
            Intent::Ignore => do_nothing_plan(
                &event.tenant_id,
                &event.room_id,
                &event.event_id,
                "planner_ignore",
            ),
            Intent::Reply | Intent::Message => match requested_action_mode(&event) {
                "start_agent_job" => start_agent_job_plan(&event, intent, &authz.reason_code),
                "request_approval" => request_approval_plan(&event, intent, &authz.reason_code),
                _ => request_generation_plan(&event, intent, &authz.reason_code),
            },
        };

        if matches!(plan.actions[0].action_type, ActionType::RequestApproval) {
            let expires_at = server_now
                + chrono::Duration::milliseconds(self.cfg.planner.approval_timeout_ms as i64);
            let approval_id = format!("approval:{}", event.event_id);
            plan.actions[0].payload.insert(
                "approval_id".to_string(),
                Value::String(approval_id.clone()),
            );
            plan.actions[0].payload.insert(
                "expires_at".to_string(),
                Value::String(expires_at.to_rfc3339()),
            );
            plan.actions[0]
                .target
                .insert("approval_id".to_string(), Value::String(approval_id));
        }
        validate_response_plan(&plan)?;

        let mut store = self.store.lock().await;
        if matches!(plan.actions[0].action_type, ActionType::RequestGeneration) {
            let action = &plan.actions[0];
            let mut room_state = store.get_room(&room_key);
            room_state.generating = true;
            room_state.pending_queue_size += 1;
            store
                .save_room(&room_key, &room_state)
                .map_err(|e| e.to_string())?;

            store
                .save_pending(
                    pending_key(&event.tenant_id, &action.action_id),
                    PendingGeneration {
                        tenant_id: event.tenant_id.clone(),
                        room_id: event.room_id.clone(),
                        action_id: action.action_id.clone(),
                        reply_to: event.content.reply_to.clone(),
                        intent,
                    },
                )
                .map_err(|e| e.to_string())?;
        }

        store
            .increment_tenant_rate(&event.tenant_id, tenant_bucket)
            .map_err(|e| e.to_string())?;

        store
            .save_idempotency(key.clone(), &plan)
            .map_err(|e| e.to_string())?;
        store
            .save_event_payload(&key, &incoming_hash)
            .map_err(|e| e.to_string())?;
        drop(store);

        self.audit
            .append(
                AuditRecord::new(
                    &event.tenant_id,
                    &event.event_id,
                    "process_event",
                    "ok",
                    action_name(&plan.actions[0]),
                    Some(plan.plan_id.clone()),
                )
                .with_trace(DecisionTrace {
                    gate: Some(StageDecision {
                        result: "allow".to_string(),
                        reason_code: "gate_allow".to_string(),
                    }),
                    authz: if self.cfg.audit.include_authz_decision {
                        Some(AuthzDecisionTrace {
                            result: "allow".to_string(),
                            reason_code: authz.reason_code.clone(),
                            policy_version: authz.policy_version.clone(),
                        })
                    } else {
                        None
                    },
                    planner: Some(PlannerDecisionTrace {
                        reply_policy: planner_cfg.reply_policy,
                        chosen_intent: intent_name(intent).to_string(),
                        seed: planner_seed,
                        sampled_probability,
                    }),
                }),
            )
            .await
            .map_err(|e| format!("internal.audit_write_failed: {e}"))?;
        Ok(plan)
    }

    async fn process_generation(&self, input: GenerationResult) -> Result<ResponsePlan, String> {
        if input.v != CONTRACT_VERSION {
            return Err("v must be 1".to_string());
        }
        if input.tenant_id.is_empty() || input.plan_id.is_empty() || input.action_id.is_empty() {
            return Err("tenant_id, plan_id, action_id are required".to_string());
        }

        let mut store = self.store.lock().await;
        let key = pending_key(&input.tenant_id, &input.action_id);
        let pending = match store.take_pending(&key).map_err(|e| e.to_string())? {
            Some(v) => v,
            None => {
                drop(store);
                let plan = do_nothing_plan(
                    &input.tenant_id,
                    "",
                    &input.action_id,
                    "generation_unknown_action",
                );
                self.audit
                    .append(AuditRecord::new(
                        &input.tenant_id,
                        &input.action_id,
                        "generation_result",
                        "no_pending_action",
                        "generation_unknown_action",
                        Some(plan.plan_id.clone()),
                    ))
                    .await
                    .map_err(|e| format!("internal.audit_write_failed: {e}"))?;
                return Ok(plan);
            }
        };

        let pending_room_key = room_key(&pending.tenant_id, &pending.room_id);
        let mut room_state = store.get_room(&pending_room_key);
        if room_state.pending_queue_size > 0 {
            room_state.pending_queue_size -= 1;
        }
        room_state.generating = room_state.pending_queue_size > 0;
        store
            .save_room(&pending_room_key, &room_state)
            .map_err(|e| e.to_string())?;
        drop(store);

        let plan = send_plan(
            &pending.tenant_id,
            &pending.room_id,
            &pending.action_id,
            &input.text,
            pending.reply_to.as_deref(),
        );
        validate_response_plan(&plan)?;
        {
            let mut store = self.store.lock().await;
            store
                .index_plan_actions(&pending.tenant_id, &plan)
                .map_err(|e| e.to_string())?;
        }

        self.audit
            .append(AuditRecord::new(
                &input.tenant_id,
                &input.action_id,
                "generation_result",
                "ok",
                action_name(&plan.actions[0]),
                Some(plan.plan_id.clone()),
            ))
            .await
            .map_err(|e| format!("internal.audit_write_failed: {e}"))?;
        Ok(plan)
    }

    async fn process_job_status(&self, input: JobStatusEvent) -> Result<ResponsePlan, String> {
        if input.v != CONTRACT_VERSION {
            return Err("v must be 1".to_string());
        }
        if input.event_id.is_empty() || input.tenant_id.is_empty() || input.job_id.is_empty() {
            return Err("event_id, tenant_id, job_id are required".to_string());
        }
        parse_event_ts(&input.ts).ok_or_else(|| "ts must be RFC3339".to_string())?;
        if !matches!(
            input.status.as_str(),
            "started" | "heartbeat" | "completed" | "failed" | "cancelled"
        ) {
            return Err("invalid job status".to_string());
        }

        let event_key = event_key(&input.tenant_id, &input.event_id);
        let payload_json = canonical_json_string(
            &serde_json::to_value(&input).map_err(|e| format!("validation_error: {e}"))?,
        );

        if let Some(existing) = {
            let store = self.store.lock().await;
            store.get_idempotency(&event_key)
        } {
            let maybe_existing_payload = {
                let store = self.store.lock().await;
                store.get_event_payload(&event_key)
            }
            .map_err(|e| format!("validation_error: {e}"))?;
            if let Some(existing_payload) = maybe_existing_payload {
                if existing_payload != payload_json {
                    return Err(
                        "conflict.payload_mismatch: duplicate event_id has different payload"
                            .to_string(),
                    );
                }
            }
            return Ok(existing);
        }

        let current = {
            let store = self.store.lock().await;
            store.get_job_state(&input.tenant_id, &input.job_id)
        }
        .map_err(|e| format!("validation_error: {e}"))?;
        if !is_valid_job_transition(current.as_ref().map(|v| v.status.as_str()), &input.status) {
            return Err(format!(
                "conflict.invalid_transition: job status transition rejected ({:?} -> {})",
                current.as_ref().map(|v| v.status.as_str()),
                input.status
            ));
        }

        {
            let mut store = self.store.lock().await;
            store
                .save_job_state(
                    &input.tenant_id,
                    &input.job_id,
                    &input.status,
                    input.reason_code.as_deref(),
                )
                .map_err(|e| format!("validation_error: {e}"))?;
        }

        let plan = do_nothing_plan(
            &input.tenant_id,
            "",
            &input.event_id,
            &format!("job_status_{}", input.status),
        );
        {
            let mut store = self.store.lock().await;
            store
                .save_idempotency(event_key.clone(), &plan)
                .map_err(|e| format!("validation_error: {e}"))?;
            store
                .save_event_payload(&event_key, &payload_json)
                .map_err(|e| format!("validation_error: {e}"))?;
        }

        self.audit
            .append(
                AuditRecord::new(
                    &input.tenant_id,
                    &input.event_id,
                    "job_event",
                    "recorded",
                    &format!("job_status_{}", input.status),
                    Some(plan.plan_id.clone()),
                )
                .with_trace(DecisionTrace {
                    gate: None,
                    authz: None,
                    planner: None,
                }),
            )
            .await
            .map_err(|e| format!("internal.audit_write_failed: {e}"))?;
        Ok(plan)
    }

    async fn process_job_cancel(&self, input: JobCancelRequest) -> Result<ResponsePlan, String> {
        if input.v != CONTRACT_VERSION {
            return Err("v must be 1".to_string());
        }
        if input.event_id.is_empty() || input.tenant_id.is_empty() || input.job_id.is_empty() {
            return Err("event_id, tenant_id, job_id are required".to_string());
        }
        parse_event_ts(&input.ts).ok_or_else(|| "ts must be RFC3339".to_string())?;

        if let Some(existing) = {
            let store = self.store.lock().await;
            store.get_idempotency(&event_key(&input.tenant_id, &input.event_id))
        } {
            return Ok(existing);
        }

        {
            let mut store = self.store.lock().await;
            store
                .save_job_state(
                    &input.tenant_id,
                    &input.job_id,
                    "cancelled",
                    input.reason_code.as_deref(),
                )
                .map_err(|e| e.to_string())?;
        }

        let plan = do_nothing_plan(&input.tenant_id, "", &input.event_id, "job_cancelled");
        {
            let mut store = self.store.lock().await;
            store
                .save_idempotency(event_key(&input.tenant_id, &input.event_id), &plan)
                .map_err(|e| e.to_string())?;
        }

        self.audit
            .append(AuditRecord::new(
                &input.tenant_id,
                &input.event_id,
                "job_cancel",
                "recorded",
                "job_cancelled",
                Some(plan.plan_id.clone()),
            ))
            .await
            .map_err(|e| format!("internal.audit_write_failed: {e}"))?;
        Ok(plan)
    }

    async fn process_approval_event(&self, input: ApprovalEvent) -> Result<ResponsePlan, String> {
        if input.v != CONTRACT_VERSION {
            return Err("v must be 1".to_string());
        }
        if input.event_id.is_empty() || input.tenant_id.is_empty() || input.approval_id.is_empty() {
            return Err("event_id, tenant_id, approval_id are required".to_string());
        }
        parse_event_ts(&input.ts).ok_or_else(|| "ts must be RFC3339".to_string())?;
        if !matches!(
            input.status.as_str(),
            "requested" | "approved" | "rejected" | "expired"
        ) {
            return Err("invalid approval status".to_string());
        }

        let event_key = event_key(&input.tenant_id, &input.event_id);
        let payload_json = canonical_json_string(
            &serde_json::to_value(&input).map_err(|e| format!("validation_error: {e}"))?,
        );

        if let Some(existing) = {
            let store = self.store.lock().await;
            store.get_idempotency(&event_key)
        } {
            let maybe_existing_payload = {
                let store = self.store.lock().await;
                store.get_event_payload(&event_key)
            }
            .map_err(|e| format!("validation_error: {e}"))?;
            if let Some(existing_payload) = maybe_existing_payload {
                if existing_payload != payload_json {
                    return Err(
                        "conflict.payload_mismatch: duplicate event_id has different payload"
                            .to_string(),
                    );
                }
            }
            return Ok(existing);
        }

        let current = {
            let store = self.store.lock().await;
            store.get_approval_state(&input.tenant_id, &input.approval_id)
        }
        .map_err(|e| format!("validation_error: {e}"))?;
        if !is_valid_approval_transition(current.as_ref().map(|v| v.status.as_str()), &input.status)
        {
            return Err(format!(
                "conflict.invalid_transition: approval status transition rejected ({:?} -> {})",
                current.as_ref().map(|v| v.status.as_str()),
                input.status
            ));
        }

        {
            let mut store = self.store.lock().await;
            store
                .save_approval_state(
                    &input.tenant_id,
                    &input.approval_id,
                    &input.status,
                    input.reason_code.as_deref(),
                )
                .map_err(|e| format!("validation_error: {e}"))?;
        }

        let reason = format!("approval_{}", input.status);
        let mut plan = do_nothing_plan(&input.tenant_id, "", &input.event_id, &reason);
        if input.status == "expired" && self.cfg.planner.approval_escalation_on_expired {
            plan.debug.insert(
                "escalation".to_string(),
                Value::String("notify_human".to_string()),
            );
        }

        {
            let mut store = self.store.lock().await;
            store
                .save_idempotency(event_key.clone(), &plan)
                .map_err(|e| format!("validation_error: {e}"))?;
            store
                .save_event_payload(&event_key, &payload_json)
                .map_err(|e| format!("validation_error: {e}"))?;
        }

        self.audit
            .append(AuditRecord::new(
                &input.tenant_id,
                &input.event_id,
                "approval_event",
                "recorded",
                &reason,
                Some(plan.plan_id.clone()),
            ))
            .await
            .map_err(|e| format!("internal.audit_write_failed: {e}"))?;
        Ok(plan)
    }
}

async fn healthz() -> (StatusCode, &'static str) {
    (StatusCode::OK, "ok")
}

async fn contracts(State(state): State<AppState>) -> Json<Value> {
    Json((*state.contracts_metadata).clone())
}

async fn job_state(
    State(state): State<AppState>,
    Path((tenant_id, job_id)): Path<(String, String)>,
) -> Result<Json<StateResponse>, (StatusCode, Json<Value>)> {
    if tenant_id.is_empty() || job_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(
                json!({"error": {"code":"validation_error","message":"tenant_id and job_id are required"}}),
            ),
        ));
    }

    let entry = {
        let store = state.store.lock().await;
        store.get_job_state(&tenant_id, &job_id)
    }
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"code":"validation_error","message": e}})),
        )
    })?;

    match entry {
        Some(v) => Ok(Json(StateResponse {
            id: job_id,
            tenant_id,
            status: v.status,
            reason_code: v.reason_code,
            updated_at: v.updated_at,
        })),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": {"code":"not_found","message":"job state not found"}})),
        )),
    }
}

async fn approval_state(
    State(state): State<AppState>,
    Path((tenant_id, approval_id)): Path<(String, String)>,
) -> Result<Json<StateResponse>, (StatusCode, Json<Value>)> {
    if tenant_id.is_empty() || approval_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(
                json!({"error": {"code":"validation_error","message":"tenant_id and approval_id are required"}}),
            ),
        ));
    }

    let entry = {
        let store = state.store.lock().await;
        store.get_approval_state(&tenant_id, &approval_id)
    }
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"code":"validation_error","message": e}})),
        )
    })?;

    match entry {
        Some(v) => Ok(Json(StateResponse {
            id: approval_id,
            tenant_id,
            status: v.status,
            reason_code: v.reason_code,
            updated_at: v.updated_at,
        })),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": {"code":"not_found","message":"approval state not found"}})),
        )),
    }
}

async fn events(
    State(state): State<AppState>,
    Json(event): Json<Event>,
) -> Result<Json<ResponsePlan>, (StatusCode, Json<Value>)> {
    state
        .process_event(event)
        .await
        .map(Json)
        .map_err(api_error)
}

async fn generations(
    State(state): State<AppState>,
    Json(input): Json<GenerationResult>,
) -> Result<Json<ResponsePlan>, (StatusCode, Json<Value>)> {
    state
        .process_generation(input)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": {"code":"validation_error","message": e}})),
            )
        })
}

async fn job_events(
    State(state): State<AppState>,
    Json(input): Json<JobStatusEvent>,
) -> Result<Json<ResponsePlan>, (StatusCode, Json<Value>)> {
    state
        .process_job_status(input)
        .await
        .map(Json)
        .map_err(api_error)
}

async fn job_cancel(
    State(state): State<AppState>,
    Json(input): Json<JobCancelRequest>,
) -> Result<Json<ResponsePlan>, (StatusCode, Json<Value>)> {
    state
        .process_job_cancel(input)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": {"code":"validation_error","message": e}})),
            )
        })
}

async fn approval_events(
    State(state): State<AppState>,
    Json(input): Json<ApprovalEvent>,
) -> Result<Json<ResponsePlan>, (StatusCode, Json<Value>)> {
    state
        .process_approval_event(input)
        .await
        .map(Json)
        .map_err(api_error)
}

async fn action_results(
    State(state): State<AppState>,
    Json(input): Json<ActionResult>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    if input.v != CONTRACT_VERSION {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"code":"validation_error","message":"v must be 1"}})),
        ));
    }
    if input.tenant_id.is_empty() || input.plan_id.is_empty() || input.action_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(
                json!({"error": {"code":"validation_error","message":"tenant_id, plan_id and action_id are required"}}),
            ),
        ));
    }
    if parse_event_ts(&input.ts).is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"code":"validation_error","message":"ts must be RFC3339"}})),
        ));
    }
    if !matches!(input.status.as_str(), "succeeded" | "failed" | "skipped") {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(
                json!({"error": {"code":"validation_error","message":"status must be succeeded, failed, or skipped"}}),
            ),
        ));
    }

    let ingested_at = Utc::now();
    let payload = serde_json::to_value(&input).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"code":"validation_error","message": e.to_string()}})),
        )
    })?;
    let payload_json = canonical_json_string(&payload);
    let idempotency_key =
        action_result_store_key(&input.tenant_id, &input.plan_id, &input.action_id);
    let context = {
        let store = state.store.lock().await;
        store
            .get_action_context(&input.tenant_id, &input.action_id)
            .map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": {"code":"validation_error","message": e}})),
                )
            })?
    };

    let mut plan_id = input.plan_id.clone();
    let mut action_type = None;
    let mut room_id = None;
    if let Some(ctx) = context {
        if plan_id.is_empty() {
            plan_id = ctx.plan_id;
        }
        action_type = Some(ctx.action_type);
        room_id = Some(ctx.room_id);
    }

    let record = ActionResultRecord {
        tenant_id: input.tenant_id.clone(),
        plan_id,
        action_id: input.action_id.clone(),
        status: input.status.clone(),
        ts: input.ts.clone(),
        provider_message_id: input.provider_message_id.clone(),
        reason_code: input.reason_code.clone(),
        error: input
            .error
            .as_ref()
            .map(|v| serde_json::to_value(v).unwrap_or(Value::Null)),
        idempotency_key,
        payload_json,
        ingested_at: ingested_at.to_rfc3339(),
        action_type,
        room_id,
    };

    let ingest = {
        let mut store = state.store.lock().await;
        store.ingest_action_result(record).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": {"code":"validation_error","message": e}})),
            )
        })?
    };

    let (stored, result_label) = match ingest {
        ActionResultIngest::Inserted(v) => (v, "recorded"),
        ActionResultIngest::Duplicate(v) => (v, "idempotency_hit"),
        ActionResultIngest::Conflict(code) => {
            if let Err(err) = state
                .audit
                .append(AuditRecord::new(
                    &input.tenant_id,
                    &input.action_id,
                    "action_result",
                    "rejected",
                    &code,
                    Some(input.plan_id.clone()),
                ))
                .await
            {
                return Err(internal_error_response(err));
            }
            return Err((
                StatusCode::CONFLICT,
                Json(json!({"error": {"code":code,"message":"action result conflict"}})),
            ));
        }
    };

    if result_label == "recorded"
        && stored.status == "succeeded"
        && matches!(
            stored.action_type.as_deref(),
            Some("send_message") | Some("send_reply")
        )
    {
        if let Some(ref room_id) = stored.room_id {
            let room_key = room_key(&stored.tenant_id, room_id);
            let mut store = state.store.lock().await;
            let mut room = store.get_room(&room_key);
            room.last_send_at = Some(ingested_at);
            store.save_room(&room_key, &room).map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": {"code":"validation_error","message": e}})),
                )
            })?;
        }
    }

    let reason = input
        .reason_code
        .as_deref()
        .filter(|v| !v.is_empty())
        .unwrap_or(match input.status.as_str() {
            "succeeded" => "action_result_succeeded",
            "failed" => "action_result_failed",
            _ => "action_result_skipped",
        });

    state
        .audit
        .append(AuditRecord::new(
            &input.tenant_id,
            &input.action_id,
            "action_result",
            result_label,
            reason,
            Some(stored.plan_id),
        ))
        .await
        .map_err(internal_error_response)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn action_result_state(
    State(state): State<AppState>,
    Path((tenant_id, plan_id, action_id)): Path<(String, String, String)>,
) -> Result<Json<ActionResult>, (StatusCode, Json<Value>)> {
    if tenant_id.is_empty() || plan_id.is_empty() || action_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(
                json!({"error": {"code":"validation_error","message":"tenant_id, plan_id, and action_id are required"}}),
            ),
        ));
    }

    let record = {
        let store = state.store.lock().await;
        store.get_action_result(&tenant_id, &plan_id, &action_id)
    }
    .map_err(validation_error_response)?;

    match record {
        Some(v) => Ok(Json(ActionResult {
            v: CONTRACT_VERSION,
            plan_id: v.plan_id,
            action_id: v.action_id,
            tenant_id: v.tenant_id,
            status: v.status,
            ts: v.ts,
            provider_message_id: v.provider_message_id,
            reason_code: v.reason_code,
            error: v.error.map(action_result_error_from_value),
        })),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": {"code":"not_found","message":"action result not found"}})),
        )),
    }
}

#[derive(Default)]
struct MemoryStore {
    idempotency: HashMap<String, ResponsePlan>,
    event_payloads: HashMap<String, String>,
    rooms: HashMap<String, RoomState>,
    pending: HashMap<String, PendingGeneration>,
    tenant_rate: HashMap<String, HashMap<i64, usize>>,
    job_states: HashMap<String, StateEntry>,
    approval_states: HashMap<String, StateEntry>,
    action_index: HashMap<String, ActionContext>,
    action_results_by_key: HashMap<String, ActionResultRecord>,
}

enum StoreBackend {
    Memory(Box<MemoryStore>),
    Sqlite(SqliteStore),
}

struct SqliteStore {
    conn: Connection,
}

#[derive(Debug, Clone)]
struct PendingGeneration {
    tenant_id: String,
    room_id: String,
    action_id: String,
    reply_to: Option<String>,
    #[allow(dead_code)]
    intent: Intent,
}

#[derive(Debug, Clone)]
struct StateEntry {
    status: String,
    reason_code: Option<String>,
    updated_at: String,
}

#[derive(Debug, Clone)]
struct ActionContext {
    plan_id: String,
    action_type: String,
    room_id: String,
}

#[derive(Debug, Clone)]
struct ActionResultRecord {
    tenant_id: String,
    plan_id: String,
    action_id: String,
    status: String,
    ts: String,
    provider_message_id: Option<String>,
    reason_code: Option<String>,
    error: Option<Value>,
    idempotency_key: String,
    payload_json: String,
    ingested_at: String,
    action_type: Option<String>,
    room_id: Option<String>,
}

enum ActionResultIngest {
    Inserted(ActionResultRecord),
    Duplicate(ActionResultRecord),
    Conflict(String),
}

#[derive(Serialize)]
struct StateResponse {
    id: String,
    tenant_id: String,
    status: String,
    reason_code: Option<String>,
    updated_at: String,
}

impl StoreBackend {
    fn get_idempotency(&self, key: &str) -> Option<ResponsePlan> {
        match self {
            StoreBackend::Memory(store) => store.idempotency.get(key).cloned(),
            StoreBackend::Sqlite(store) => store.get_idempotency(key).ok().flatten(),
        }
    }

    fn save_idempotency(&mut self, key: String, plan: &ResponsePlan) -> Result<(), String> {
        match self {
            StoreBackend::Memory(store) => {
                store.idempotency.insert(key, plan.clone());
                Self::index_plan_actions_memory(store, plan);
                Ok(())
            }
            StoreBackend::Sqlite(store) => store.save_idempotency(&key, plan),
        }
    }

    fn index_plan_actions(&mut self, tenant_id: &str, plan: &ResponsePlan) -> Result<(), String> {
        match self {
            StoreBackend::Memory(store) => {
                Self::index_plan_actions_memory(store, plan);
                Ok(())
            }
            StoreBackend::Sqlite(store) => store.index_plan_actions(tenant_id, plan),
        }
    }

    fn index_plan_actions_memory(store: &mut MemoryStore, plan: &ResponsePlan) {
        for action in &plan.actions {
            let key = action_index_key(&plan.tenant_id, &action.action_id);
            store.action_index.insert(
                key,
                ActionContext {
                    plan_id: plan.plan_id.clone(),
                    action_type: action_name(action).to_string(),
                    room_id: plan.room_id.clone(),
                },
            );
        }
    }

    fn get_room(&self, key: &str) -> RoomState {
        match self {
            StoreBackend::Memory(store) => store.rooms.get(key).cloned().unwrap_or_default(),
            StoreBackend::Sqlite(store) => store.get_room(key).unwrap_or_default(),
        }
    }

    fn get_event_payload(&self, event_key: &str) -> Result<Option<String>, String> {
        match self {
            StoreBackend::Memory(store) => Ok(store.event_payloads.get(event_key).cloned()),
            StoreBackend::Sqlite(store) => store.get_event_payload(event_key),
        }
    }

    fn save_event_payload(&mut self, event_key: &str, payload_json: &str) -> Result<(), String> {
        match self {
            StoreBackend::Memory(store) => {
                store
                    .event_payloads
                    .insert(event_key.to_string(), payload_json.to_string());
                Ok(())
            }
            StoreBackend::Sqlite(store) => store.save_event_payload(event_key, payload_json),
        }
    }

    fn save_room(&mut self, key: &str, room: &RoomState) -> Result<(), String> {
        match self {
            StoreBackend::Memory(store) => {
                store.rooms.insert(key.to_string(), room.clone());
                Ok(())
            }
            StoreBackend::Sqlite(store) => store.save_room(key, room),
        }
    }

    fn get_tenant_rate_count(&self, tenant_id: &str, bucket: i64) -> usize {
        match self {
            StoreBackend::Memory(store) => store
                .tenant_rate
                .get(tenant_id)
                .and_then(|m| m.get(&bucket))
                .copied()
                .unwrap_or(0),
            StoreBackend::Sqlite(store) => {
                store.get_tenant_rate_count(tenant_id, bucket).unwrap_or(0)
            }
        }
    }

    fn increment_tenant_rate(&mut self, tenant_id: &str, bucket: i64) -> Result<(), String> {
        match self {
            StoreBackend::Memory(store) => {
                store
                    .tenant_rate
                    .entry(tenant_id.to_string())
                    .or_default()
                    .entry(bucket)
                    .and_modify(|v| *v += 1)
                    .or_insert(1);
                Ok(())
            }
            StoreBackend::Sqlite(store) => store.increment_tenant_rate(tenant_id, bucket),
        }
    }

    fn save_pending(&mut self, key: String, pending: PendingGeneration) -> Result<(), String> {
        match self {
            StoreBackend::Memory(store) => {
                store.pending.insert(key, pending);
                Ok(())
            }
            StoreBackend::Sqlite(store) => store.save_pending(&key, &pending),
        }
    }

    fn take_pending(&mut self, key: &str) -> Result<Option<PendingGeneration>, String> {
        match self {
            StoreBackend::Memory(store) => Ok(store.pending.remove(key)),
            StoreBackend::Sqlite(store) => store.take_pending(key),
        }
    }

    fn save_job_state(
        &mut self,
        tenant_id: &str,
        job_id: &str,
        status: &str,
        reason_code: Option<&str>,
    ) -> Result<(), String> {
        let key = format!("{tenant_id}:{job_id}");
        let updated_at = Utc::now().to_rfc3339();
        match self {
            StoreBackend::Memory(store) => {
                store.job_states.insert(
                    key,
                    StateEntry {
                        status: status.to_string(),
                        reason_code: reason_code.map(|v| v.to_string()),
                        updated_at,
                    },
                );
                Ok(())
            }
            StoreBackend::Sqlite(store) => {
                store.save_job_state(tenant_id, job_id, status, reason_code)
            }
        }
    }

    fn save_approval_state(
        &mut self,
        tenant_id: &str,
        approval_id: &str,
        status: &str,
        reason_code: Option<&str>,
    ) -> Result<(), String> {
        let key = format!("{tenant_id}:{approval_id}");
        let updated_at = Utc::now().to_rfc3339();
        match self {
            StoreBackend::Memory(store) => {
                store.approval_states.insert(
                    key,
                    StateEntry {
                        status: status.to_string(),
                        reason_code: reason_code.map(|v| v.to_string()),
                        updated_at,
                    },
                );
                Ok(())
            }
            StoreBackend::Sqlite(store) => {
                store.save_approval_state(tenant_id, approval_id, status, reason_code)
            }
        }
    }

    fn get_job_state(&self, tenant_id: &str, job_id: &str) -> Result<Option<StateEntry>, String> {
        let key = format!("{tenant_id}:{job_id}");
        match self {
            StoreBackend::Memory(store) => Ok(store.job_states.get(&key).cloned()),
            StoreBackend::Sqlite(store) => store.get_job_state(tenant_id, job_id),
        }
    }

    fn get_approval_state(
        &self,
        tenant_id: &str,
        approval_id: &str,
    ) -> Result<Option<StateEntry>, String> {
        let key = format!("{tenant_id}:{approval_id}");
        match self {
            StoreBackend::Memory(store) => Ok(store.approval_states.get(&key).cloned()),
            StoreBackend::Sqlite(store) => store.get_approval_state(tenant_id, approval_id),
        }
    }

    fn get_action_context(
        &self,
        tenant_id: &str,
        action_id: &str,
    ) -> Result<Option<ActionContext>, String> {
        match self {
            StoreBackend::Memory(store) => Ok(store
                .action_index
                .get(&action_index_key(tenant_id, action_id))
                .cloned()),
            StoreBackend::Sqlite(store) => store.get_action_context(tenant_id, action_id),
        }
    }

    fn ingest_action_result(
        &mut self,
        mut record: ActionResultRecord,
    ) -> Result<ActionResultIngest, String> {
        let action_result_key =
            action_result_store_key(&record.tenant_id, &record.plan_id, &record.action_id);
        match self {
            StoreBackend::Memory(store) => {
                if let Some(existing) = store.action_results_by_key.get(&record.idempotency_key) {
                    if existing.payload_json == record.payload_json {
                        return Ok(ActionResultIngest::Duplicate(existing.clone()));
                    }
                    return Ok(ActionResultIngest::Conflict(
                        "conflict.payload_mismatch".to_string(),
                    ));
                }

                store
                    .action_results_by_key
                    .insert(record.idempotency_key.clone(), record.clone());
                store
                    .action_results_by_key
                    .insert(action_result_key, record.clone());
                Ok(ActionResultIngest::Inserted(record))
            }
            StoreBackend::Sqlite(store) => {
                if record.action_type.is_none() || record.room_id.is_none() {
                    if let Some(ctx) =
                        store.get_action_context(&record.tenant_id, &record.action_id)?
                    {
                        record.action_type = Some(ctx.action_type);
                        record.room_id = Some(ctx.room_id);
                    }
                }
                store.ingest_action_result(record)
            }
        }
    }

    fn get_action_result(
        &self,
        tenant_id: &str,
        plan_id: &str,
        action_id: &str,
    ) -> Result<Option<ActionResultRecord>, String> {
        let key = action_result_store_key(tenant_id, plan_id, action_id);
        match self {
            StoreBackend::Memory(store) => Ok(store.action_results_by_key.get(&key).cloned()),
            StoreBackend::Sqlite(store) => store.get_action_result(tenant_id, plan_id, action_id),
        }
    }
}

impl SqliteStore {
    fn new(path: &str) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS idempotency (
                event_key TEXT PRIMARY KEY,
                plan_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS event_payloads (
                event_key TEXT PRIMARY KEY,
                payload_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS rooms (
                room_key TEXT PRIMARY KEY,
                generating INTEGER NOT NULL,
                pending_queue_size INTEGER NOT NULL,
                last_send_at TEXT
            );
            CREATE TABLE IF NOT EXISTS pending_generations (
                pending_key TEXT PRIMARY KEY,
                tenant_id TEXT NOT NULL,
                room_id TEXT NOT NULL,
                action_id TEXT NOT NULL,
                reply_to TEXT,
                intent TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS tenant_rate (
                tenant_id TEXT NOT NULL,
                bucket INTEGER NOT NULL,
                count INTEGER NOT NULL,
                PRIMARY KEY (tenant_id, bucket)
            );
            CREATE TABLE IF NOT EXISTS job_states (
                tenant_id TEXT NOT NULL,
                job_id TEXT NOT NULL,
                status TEXT NOT NULL,
                reason_code TEXT,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (tenant_id, job_id)
            );
            CREATE TABLE IF NOT EXISTS approval_states (
                tenant_id TEXT NOT NULL,
                approval_id TEXT NOT NULL,
                status TEXT NOT NULL,
                reason_code TEXT,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (tenant_id, approval_id)
            );
            CREATE TABLE IF NOT EXISTS action_index (
                tenant_id TEXT NOT NULL,
                action_id TEXT NOT NULL,
                plan_id TEXT NOT NULL,
                action_type TEXT NOT NULL,
                room_id TEXT NOT NULL,
                PRIMARY KEY (tenant_id, action_id)
            );
            CREATE TABLE IF NOT EXISTS action_results (
                tenant_id TEXT NOT NULL,
                action_id TEXT NOT NULL,
                plan_id TEXT NOT NULL,
                status TEXT NOT NULL,
                ts TEXT NOT NULL,
                provider_message_id TEXT,
                reason_code TEXT,
                error_json TEXT,
                idempotency_key TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                ingested_at TEXT NOT NULL,
                action_type TEXT,
                room_id TEXT,
                PRIMARY KEY (tenant_id, plan_id, action_id),
                UNIQUE (tenant_id, idempotency_key)
            );
            ",
        )
        .map_err(|e| e.to_string())?;
        Ok(Self { conn })
    }

    fn get_idempotency(&self, key: &str) -> Result<Option<ResponsePlan>, String> {
        let plan_json: Option<String> = self
            .conn
            .query_row(
                "SELECT plan_json FROM idempotency WHERE event_key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        match plan_json {
            Some(v) => {
                let plan: ResponsePlan = serde_json::from_str(&v).map_err(|e| e.to_string())?;
                Ok(Some(plan))
            }
            None => Ok(None),
        }
    }

    fn save_idempotency(&mut self, key: &str, plan: &ResponsePlan) -> Result<(), String> {
        let json = serde_json::to_string(plan).map_err(|e| e.to_string())?;
        self.conn
            .execute(
                "INSERT OR REPLACE INTO idempotency(event_key, plan_json) VALUES (?1, ?2)",
                params![key, json],
            )
            .map_err(|e| e.to_string())?;
        self.index_plan_actions(&plan.tenant_id, plan)?;
        Ok(())
    }

    fn get_event_payload(&self, event_key: &str) -> Result<Option<String>, String> {
        self.conn
            .query_row(
                "SELECT payload_json FROM event_payloads WHERE event_key = ?1",
                params![event_key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    fn save_event_payload(&mut self, event_key: &str, payload_json: &str) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO event_payloads(event_key, payload_json) VALUES (?1, ?2)",
                params![event_key, payload_json],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn index_plan_actions(&mut self, tenant_id: &str, plan: &ResponsePlan) -> Result<(), String> {
        for action in &plan.actions {
            self.conn
                .execute(
                    "
                    INSERT INTO action_index(tenant_id, action_id, plan_id, action_type, room_id)
                    VALUES (?1, ?2, ?3, ?4, ?5)
                    ON CONFLICT(tenant_id, action_id) DO UPDATE SET
                        plan_id=excluded.plan_id,
                        action_type=excluded.action_type,
                        room_id=excluded.room_id
                    ",
                    params![
                        tenant_id,
                        action.action_id,
                        plan.plan_id,
                        action_name(action),
                        plan.room_id
                    ],
                )
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn get_room(&self, key: &str) -> Result<RoomState, String> {
        let row = self
            .conn
            .query_row(
                "SELECT generating, pending_queue_size, last_send_at FROM rooms WHERE room_key = ?1",
                params![key],
                |row| {
                    let generating: i64 = row.get(0)?;
                    let pending_queue_size: i64 = row.get(1)?;
                    let last_send_at: Option<String> = row.get(2)?;
                    Ok((generating, pending_queue_size, last_send_at))
                },
            )
            .optional()
            .map_err(|e| e.to_string())?;

        match row {
            Some((generating, pending_queue_size, last_send_at)) => Ok(RoomState {
                generating: generating != 0,
                pending_queue_size: pending_queue_size as usize,
                last_send_at: last_send_at.and_then(|v| parse_event_ts(&v)),
            }),
            None => Ok(RoomState::default()),
        }
    }

    fn save_room(&mut self, key: &str, room: &RoomState) -> Result<(), String> {
        let last_send_at = room.last_send_at.map(|v| v.to_rfc3339());
        self.conn
            .execute(
                "
                INSERT INTO rooms(room_key, generating, pending_queue_size, last_send_at)
                VALUES (?1, ?2, ?3, ?4)
                ON CONFLICT(room_key) DO UPDATE SET
                    generating=excluded.generating,
                    pending_queue_size=excluded.pending_queue_size,
                    last_send_at=excluded.last_send_at
                ",
                params![
                    key,
                    if room.generating { 1 } else { 0 },
                    room.pending_queue_size as i64,
                    last_send_at
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get_tenant_rate_count(&self, tenant_id: &str, bucket: i64) -> Result<usize, String> {
        let count: Option<i64> = self
            .conn
            .query_row(
                "SELECT count FROM tenant_rate WHERE tenant_id = ?1 AND bucket = ?2",
                params![tenant_id, bucket],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(count.unwrap_or(0) as usize)
    }

    fn increment_tenant_rate(&mut self, tenant_id: &str, bucket: i64) -> Result<(), String> {
        self.conn
            .execute(
                "
                INSERT INTO tenant_rate(tenant_id, bucket, count)
                VALUES (?1, ?2, 1)
                ON CONFLICT(tenant_id, bucket) DO UPDATE SET count = count + 1
                ",
                params![tenant_id, bucket],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn save_pending(&mut self, key: &str, pending: &PendingGeneration) -> Result<(), String> {
        self.conn
            .execute(
                "
                INSERT OR REPLACE INTO pending_generations
                (pending_key, tenant_id, room_id, action_id, reply_to, intent)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                ",
                params![
                    key,
                    pending.tenant_id,
                    pending.room_id,
                    pending.action_id,
                    pending.reply_to,
                    intent_name(pending.intent)
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn take_pending(&mut self, key: &str) -> Result<Option<PendingGeneration>, String> {
        let row = self
            .conn
            .query_row(
                "SELECT tenant_id, room_id, action_id, reply_to, intent FROM pending_generations WHERE pending_key = ?1",
                params![key],
                |row| {
                    Ok(PendingGeneration {
                        tenant_id: row.get(0)?,
                        room_id: row.get(1)?,
                        action_id: row.get(2)?,
                        reply_to: row.get(3)?,
                        intent: match row.get::<_, String>(4)?.as_str() {
                            "REPLY" => Intent::Reply,
                            "MESSAGE" => Intent::Message,
                            _ => Intent::Ignore,
                        },
                    })
                },
            )
            .optional()
            .map_err(|e| e.to_string())?;

        if row.is_some() {
            self.conn
                .execute(
                    "DELETE FROM pending_generations WHERE pending_key = ?1",
                    params![key],
                )
                .map_err(|e| e.to_string())?;
        }
        Ok(row)
    }

    fn save_job_state(
        &mut self,
        tenant_id: &str,
        job_id: &str,
        status: &str,
        reason_code: Option<&str>,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "
                INSERT INTO job_states (tenant_id, job_id, status, reason_code, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5)
                ON CONFLICT(tenant_id, job_id) DO UPDATE SET
                    status=excluded.status,
                    reason_code=excluded.reason_code,
                    updated_at=excluded.updated_at
                ",
                params![
                    tenant_id,
                    job_id,
                    status,
                    reason_code,
                    Utc::now().to_rfc3339()
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn save_approval_state(
        &mut self,
        tenant_id: &str,
        approval_id: &str,
        status: &str,
        reason_code: Option<&str>,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "
                INSERT INTO approval_states (tenant_id, approval_id, status, reason_code, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5)
                ON CONFLICT(tenant_id, approval_id) DO UPDATE SET
                    status=excluded.status,
                    reason_code=excluded.reason_code,
                    updated_at=excluded.updated_at
                ",
                params![
                    tenant_id,
                    approval_id,
                    status,
                    reason_code,
                    Utc::now().to_rfc3339()
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get_job_state(&self, tenant_id: &str, job_id: &str) -> Result<Option<StateEntry>, String> {
        self.conn
            .query_row(
                "SELECT status, reason_code, updated_at FROM job_states WHERE tenant_id = ?1 AND job_id = ?2",
                params![tenant_id, job_id],
                |row| {
                    Ok(StateEntry {
                        status: row.get(0)?,
                        reason_code: row.get(1)?,
                        updated_at: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    fn get_approval_state(
        &self,
        tenant_id: &str,
        approval_id: &str,
    ) -> Result<Option<StateEntry>, String> {
        self.conn
            .query_row(
                "SELECT status, reason_code, updated_at FROM approval_states WHERE tenant_id = ?1 AND approval_id = ?2",
                params![tenant_id, approval_id],
                |row| {
                    Ok(StateEntry {
                        status: row.get(0)?,
                        reason_code: row.get(1)?,
                        updated_at: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    fn get_action_context(
        &self,
        tenant_id: &str,
        action_id: &str,
    ) -> Result<Option<ActionContext>, String> {
        self.conn
            .query_row(
                "SELECT plan_id, action_type, room_id FROM action_index WHERE tenant_id = ?1 AND action_id = ?2",
                params![tenant_id, action_id],
                |row| {
                    Ok(ActionContext {
                        plan_id: row.get(0)?,
                        action_type: row.get(1)?,
                        room_id: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    fn get_action_result_by_idempotency(
        &self,
        tenant_id: &str,
        idempotency_key: &str,
    ) -> Result<Option<ActionResultRecord>, String> {
        self.conn
            .query_row(
                "
                SELECT tenant_id, plan_id, action_id, status, ts, provider_message_id, reason_code,
                       error_json, idempotency_key, payload_json, ingested_at, action_type, room_id
                FROM action_results
                WHERE tenant_id = ?1 AND idempotency_key = ?2
                ",
                params![tenant_id, idempotency_key],
                |row| {
                    let error_json: Option<String> = row.get(7)?;
                    Ok(ActionResultRecord {
                        tenant_id: row.get(0)?,
                        plan_id: row.get(1)?,
                        action_id: row.get(2)?,
                        status: row.get(3)?,
                        ts: row.get(4)?,
                        provider_message_id: row.get(5)?,
                        reason_code: row.get(6)?,
                        error: error_json
                            .as_deref()
                            .and_then(|v| serde_json::from_str::<Value>(v).ok()),
                        idempotency_key: row.get(8)?,
                        payload_json: row.get(9)?,
                        ingested_at: row.get(10)?,
                        action_type: row.get(11)?,
                        room_id: row.get(12)?,
                    })
                },
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    fn get_action_result(
        &self,
        tenant_id: &str,
        plan_id: &str,
        action_id: &str,
    ) -> Result<Option<ActionResultRecord>, String> {
        self.conn
            .query_row(
                "
                SELECT tenant_id, plan_id, action_id, status, ts, provider_message_id, reason_code,
                       error_json, idempotency_key, payload_json, ingested_at, action_type, room_id
                FROM action_results
                WHERE tenant_id = ?1 AND plan_id = ?2 AND action_id = ?3
                ",
                params![tenant_id, plan_id, action_id],
                |row| {
                    let error_json: Option<String> = row.get(7)?;
                    Ok(ActionResultRecord {
                        tenant_id: row.get(0)?,
                        plan_id: row.get(1)?,
                        action_id: row.get(2)?,
                        status: row.get(3)?,
                        ts: row.get(4)?,
                        provider_message_id: row.get(5)?,
                        reason_code: row.get(6)?,
                        error: error_json
                            .as_deref()
                            .and_then(|v| serde_json::from_str::<Value>(v).ok()),
                        idempotency_key: row.get(8)?,
                        payload_json: row.get(9)?,
                        ingested_at: row.get(10)?,
                        action_type: row.get(11)?,
                        room_id: row.get(12)?,
                    })
                },
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    fn insert_action_result(&mut self, record: &ActionResultRecord) -> Result<(), String> {
        self.conn
            .execute(
                "
                INSERT INTO action_results(
                    tenant_id, action_id, plan_id, status, ts, provider_message_id, reason_code,
                    error_json, idempotency_key, payload_json, ingested_at, action_type, room_id
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                ",
                params![
                    record.tenant_id,
                    record.action_id,
                    record.plan_id,
                    record.status,
                    record.ts,
                    record.provider_message_id,
                    record.reason_code,
                    record.error.as_ref().map(|v| v.to_string()),
                    record.idempotency_key,
                    record.payload_json,
                    record.ingested_at,
                    record.action_type,
                    record.room_id
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn ingest_action_result(
        &mut self,
        record: ActionResultRecord,
    ) -> Result<ActionResultIngest, String> {
        let action_result_key =
            action_result_store_key(&record.tenant_id, &record.plan_id, &record.action_id);
        if let Some(existing) =
            self.get_action_result_by_idempotency(&record.tenant_id, &record.idempotency_key)?
        {
            if existing.payload_json == record.payload_json {
                return Ok(ActionResultIngest::Duplicate(existing));
            }
            return Ok(ActionResultIngest::Conflict(
                "conflict.payload_mismatch".to_string(),
            ));
        }

        if let Some(existing) =
            self.get_action_result(&record.tenant_id, &record.plan_id, &record.action_id)?
        {
            if existing.payload_json == record.payload_json {
                return Ok(ActionResultIngest::Duplicate(existing));
            }
            return Ok(ActionResultIngest::Conflict(
                "conflict.payload_mismatch".to_string(),
            ));
        }

        if record.idempotency_key != action_result_key {
            return Ok(ActionResultIngest::Conflict(
                "conflict.payload_mismatch".to_string(),
            ));
        }

        self.insert_action_result(&record)?;
        Ok(ActionResultIngest::Inserted(record))
    }
}

#[derive(Debug, Clone)]
struct AuthzOutcome {
    allow: bool,
    reason_code: String,
    policy_version: Option<String>,
}

struct AuthzEngine {
    mode: String,
    endpoint: Option<String>,
    fail_mode: String,
    retry_max_attempts: usize,
    retry_backoff: Duration,
    circuit_breaker_failures: u64,
    circuit_breaker_open: Duration,
    cache_enabled: bool,
    cache_ttl: Duration,
    cache_max_entries: usize,
    cache: Arc<Mutex<HashMap<String, CachedDecision>>>,
    failure_streak: Arc<Mutex<u64>>,
    circuit_open_until: Arc<Mutex<Option<Instant>>>,
    client: Client,
}

#[derive(Clone)]
struct CachedDecision {
    outcome: AuthzOutcome,
    expires_at: Instant,
}

impl AuthzEngine {
    fn new(cfg: &Config) -> Result<Self, String> {
        let timeout = Duration::from_millis(cfg.authz.timeout_ms as u64);
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            mode: cfg.authz.mode.clone(),
            endpoint: cfg.authz.endpoint.clone(),
            fail_mode: cfg.authz.fail_mode.clone(),
            retry_max_attempts: cfg.authz.retry_max_attempts.max(1),
            retry_backoff: Duration::from_millis(cfg.authz.retry_backoff_ms),
            circuit_breaker_failures: cfg.authz.circuit_breaker_failures.max(1),
            circuit_breaker_open: Duration::from_millis(cfg.authz.circuit_breaker_open_ms.max(1)),
            cache_enabled: cfg.authz.cache.enabled,
            cache_ttl: Duration::from_millis(cfg.authz.cache.ttl_ms as u64),
            cache_max_entries: cfg.authz.cache.max_entries,
            cache: Arc::new(Mutex::new(HashMap::new())),
            failure_streak: Arc::new(Mutex::new(0)),
            circuit_open_until: Arc::new(Mutex::new(None)),
            client,
        })
    }

    async fn authorize(&self, event: &Event) -> AuthzOutcome {
        if self.mode == "builtin" {
            return AuthzOutcome {
                allow: true,
                reason_code: "builtin_allow_all".to_string(),
                policy_version: Some("builtin:v0".to_string()),
            };
        }

        {
            let open_until = self.circuit_open_until.lock().await;
            if let Some(until) = *open_until {
                if until > Instant::now() {
                    return self.on_failure("authz_circuit_open");
                }
            }
        }

        let key = format!(
            "{}:{}:{}:{}",
            event.tenant_id, event.actor.id, event.room_id, event.source
        );
        if self.cache_enabled {
            let cache = self.cache.lock().await;
            if let Some(cached) = cache.get(&key) {
                if cached.expires_at > Instant::now() {
                    return cached.outcome.clone();
                }
            }
        }

        let endpoint = match &self.endpoint {
            Some(v) if !v.is_empty() => v,
            _ => return self.on_failure("authz_unconfigured"),
        };

        let request = AuthZRequest {
            v: CONTRACT_VERSION,
            tenant_id: event.tenant_id.clone(),
            correlation_id: event.event_id.clone(),
            actor: event.actor.clone(),
            request: AuthZReqData {
                action: "process_event".to_string(),
                resource: AuthZResource {
                    resource_type: "room".to_string(),
                    id: event.room_id.clone(),
                    attributes: {
                        let mut m = Map::new();
                        m.insert("source".to_string(), Value::String(event.source.clone()));
                        m
                    },
                },
                context: {
                    let mut m = Map::new();
                    m.insert(
                        "event_id".to_string(),
                        Value::String(event.event_id.clone()),
                    );
                    m
                },
            },
        };

        let mut last_failure = "authz_transport_error";
        let mut decision_opt = None;
        for attempt in 0..self.retry_max_attempts {
            let response = match self.client.post(endpoint).json(&request).send().await {
                Ok(v) => v,
                Err(_) => {
                    last_failure = "authz_transport_error";
                    if attempt + 1 < self.retry_max_attempts && self.retry_backoff > Duration::ZERO
                    {
                        sleep(self.retry_backoff).await;
                    }
                    continue;
                }
            };
            if !response.status().is_success() {
                last_failure = "authz_http_error";
                if attempt + 1 < self.retry_max_attempts && self.retry_backoff > Duration::ZERO {
                    sleep(self.retry_backoff).await;
                }
                continue;
            }

            let decision: AuthZDecision = match response.json().await {
                Ok(v) => v,
                Err(_) => {
                    last_failure = "authz_contract_parse_error";
                    if attempt + 1 < self.retry_max_attempts && self.retry_backoff > Duration::ZERO
                    {
                        sleep(self.retry_backoff).await;
                    }
                    continue;
                }
            };
            if decision.v != CONTRACT_VERSION
                || (decision.decision != "allow" && decision.decision != "deny")
                || decision.policy_version.trim().is_empty()
            {
                last_failure = "authz_contract_invalid";
                break;
            }
            decision_opt = Some(decision);
            break;
        }

        let decision = match decision_opt {
            Some(v) => {
                self.record_authz_success().await;
                v
            }
            None => {
                self.record_authz_failure().await;
                return self.on_failure(last_failure);
            }
        };

        let outcome = AuthzOutcome {
            allow: decision.decision == "allow",
            reason_code: if decision.reason_code.is_empty() {
                if decision.decision == "allow" {
                    "authz_allow".to_string()
                } else {
                    "authz_deny".to_string()
                }
            } else {
                decision.reason_code
            },
            policy_version: Some(decision.policy_version),
        };

        if self.cache_enabled {
            let ttl = if decision.ttl_ms > 0 {
                Duration::from_millis(decision.ttl_ms as u64)
            } else {
                self.cache_ttl
            };
            let mut cache = self.cache.lock().await;
            if cache.len() >= self.cache_max_entries {
                cache.clear();
            }
            cache.insert(
                key,
                CachedDecision {
                    outcome: outcome.clone(),
                    expires_at: Instant::now() + ttl,
                },
            );
        }
        outcome
    }

    async fn record_authz_failure(&self) {
        let mut streak = self.failure_streak.lock().await;
        *streak += 1;
        if *streak >= self.circuit_breaker_failures {
            let mut open_until = self.circuit_open_until.lock().await;
            *open_until = Some(Instant::now() + self.circuit_breaker_open);
        }
    }

    async fn record_authz_success(&self) {
        let mut streak = self.failure_streak.lock().await;
        *streak = 0;
        let mut open_until = self.circuit_open_until.lock().await;
        *open_until = None;
    }

    fn on_failure(&self, reason: &str) -> AuthzOutcome {
        match self.fail_mode.as_str() {
            "allow" => AuthzOutcome {
                allow: true,
                reason_code: format!("{reason}_allow"),
                policy_version: None,
            },
            "fallback_builtin" => AuthzOutcome {
                allow: true,
                reason_code: format!("{reason}_fallback_builtin"),
                policy_version: Some("builtin:fallback".to_string()),
            },
            _ => AuthzOutcome {
                allow: false,
                reason_code: format!("{reason}_deny"),
                policy_version: None,
            },
        }
    }
}

struct AuditJsonl {
    file: Arc<Mutex<tokio::fs::File>>,
    immutable_mirror: Option<Arc<Mutex<tokio::fs::File>>>,
    sqlite: Option<Arc<Mutex<Connection>>>,
    last_hash: Arc<Mutex<Option<String>>>,
}

#[derive(Serialize, Deserialize, Clone)]
struct AuditRecord {
    audit_id: String,
    tenant_id: String,
    correlation_id: String,
    action: String,
    result: String,
    reason_code: String,
    ts: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    plan_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    decision_trace: Option<DecisionTrace>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prev_hash: Option<String>,
    record_hash: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct DecisionTrace {
    #[serde(skip_serializing_if = "Option::is_none")]
    gate: Option<StageDecision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    authz: Option<AuthzDecisionTrace>,
    #[serde(skip_serializing_if = "Option::is_none")]
    planner: Option<PlannerDecisionTrace>,
}

#[derive(Serialize, Deserialize, Clone)]
struct StageDecision {
    result: String,
    reason_code: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct AuthzDecisionTrace {
    result: String,
    reason_code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    policy_version: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
struct PlannerDecisionTrace {
    reply_policy: String,
    chosen_intent: String,
    seed: u64,
    sampled_probability: f64,
}

impl AuditRecord {
    fn new(
        tenant_id: &str,
        correlation_id: &str,
        action: &str,
        result: &str,
        reason_code: &str,
        plan_id: Option<String>,
    ) -> Self {
        Self {
            audit_id: format!("audit_{}", uuid::Uuid::new_v4().as_simple()),
            tenant_id: tenant_id.to_string(),
            correlation_id: correlation_id.to_string(),
            action: action.to_string(),
            result: result.to_string(),
            reason_code: reason_code.to_string(),
            ts: Utc::now().to_rfc3339(),
            plan_id,
            decision_trace: None,
            prev_hash: None,
            record_hash: String::new(),
        }
    }

    fn with_trace(mut self, trace: DecisionTrace) -> Self {
        self.decision_trace = Some(trace);
        self
    }
}

impl AuditJsonl {
    async fn new(
        path: &str,
        sqlite_path: Option<&str>,
        immutable_mirror_path: Option<&str>,
    ) -> Result<Self, String> {
        let last_hash = std::fs::read_to_string(path).ok().and_then(|text| {
            text.lines().rev().find_map(|line| {
                serde_json::from_str::<serde_json::Value>(line)
                    .ok()
                    .and_then(|v| {
                        v.get("record_hash")
                            .and_then(|hash| hash.as_str())
                            .map(|s| s.to_string())
                    })
            })
        });

        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await
            .map_err(|e| e.to_string())?;

        let immutable_mirror = match immutable_mirror_path {
            Some(path) if !path.is_empty() => Some(Arc::new(Mutex::new(
                tokio::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .await
                    .map_err(|e| e.to_string())?,
            ))),
            _ => None,
        };

        let sqlite = match sqlite_path {
            Some(path) => {
                let conn = Connection::open(path).map_err(|e| e.to_string())?;
                conn.execute_batch(
                    "
                    CREATE TABLE IF NOT EXISTS audit_records (
                        audit_id TEXT PRIMARY KEY,
                        tenant_id TEXT NOT NULL,
                        correlation_id TEXT NOT NULL,
                        action TEXT NOT NULL,
                        result TEXT NOT NULL,
                        reason_code TEXT NOT NULL,
                        ts TEXT NOT NULL,
                        plan_id TEXT,
                        record_json TEXT NOT NULL
                    );
                    ",
                )
                .map_err(|e| e.to_string())?;
                Some(Arc::new(Mutex::new(conn)))
            }
            None => None,
        };

        Ok(Self {
            file: Arc::new(Mutex::new(file)),
            immutable_mirror,
            sqlite,
            last_hash: Arc::new(Mutex::new(last_hash)),
        })
    }

    async fn append(&self, mut rec: AuditRecord) -> Result<(), String> {
        let prev_hash = { self.last_hash.lock().await.clone() };
        rec.prev_hash = prev_hash;
        let seed = serde_json::to_string(&rec).map_err(|e| e.to_string())?;
        rec.record_hash = hash_hex(seed.as_bytes());

        let mut file = self.file.lock().await;
        let line = serde_json::to_string(&rec).map_err(|e| e.to_string())?;
        use tokio::io::AsyncWriteExt;
        file.write_all(line.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        file.write_all(b"\n").await.map_err(|e| e.to_string())?;

        {
            let mut last_hash = self.last_hash.lock().await;
            *last_hash = Some(rec.record_hash.clone());
        }

        if let Some(mirror) = &self.immutable_mirror {
            let mut mirror_file = mirror.lock().await;
            mirror_file
                .write_all(line.as_bytes())
                .await
                .map_err(|e| format!("mirror write failed: {e}"))?;
            mirror_file
                .write_all(b"\n")
                .await
                .map_err(|e| format!("mirror write failed: {e}"))?;
        }

        if let Some(sqlite) = &self.sqlite {
            let conn = sqlite.lock().await;
            conn.execute(
                "
                INSERT OR REPLACE INTO audit_records
                (audit_id, tenant_id, correlation_id, action, result, reason_code, ts, plan_id, record_json)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                ",
                params![
                    rec.audit_id,
                    rec.tenant_id,
                    rec.correlation_id,
                    rec.action,
                    rec.result,
                    rec.reason_code,
                    rec.ts,
                    rec.plan_id,
                    line
                ],
            )
            .map_err(|e| e.to_string())?;
        }

        Ok(())
    }
}

fn hash_hex(input: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input);
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn generate_contracts_metadata() -> Result<Value, String> {
    let manifest = contracts_manifest_v1();
    let mut schemas = Map::new();
    for schema in &manifest.schemas {
        schemas.insert(
            schema.path.to_string(),
            Value::String(schema.sha256.to_string()),
        );
    }

    let actions = schema_enum_values("../contracts/v1/action.schema.json", "properties.type.enum")?;
    let job_events = schema_enum_values(
        "../contracts/v1/job_status_event.schema.json",
        "properties.status.enum",
    )?;
    let approval_events = schema_enum_values(
        "../contracts/v1/approval_event.schema.json",
        "properties.status.enum",
    )?;

    Ok(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "openapi_sha256": manifest.openapi_sha256,
        "contracts_set_sha256": manifest.contracts_set_sha256,
        "generated_at": manifest.generated_at,
        "actions": {
            "enabled": actions,
            "reserved": []
        },
        "inputs": {
            "job_events": job_events,
            "approval_events": approval_events
        },
        "schemas": schemas
    }))
}

fn schema_enum_values(schema_path: &str, path: &str) -> Result<Vec<String>, String> {
    let manifest = contracts_manifest_v1();
    let body = manifest
        .schemas
        .iter()
        .find(|schema| schema.path == schema_path)
        .map(|schema| schema.body)
        .ok_or_else(|| format!("missing schema: {schema_path}"))?;
    let value: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let mut current = &value;
    for part in path.split('.') {
        current = current
            .get(part)
            .ok_or_else(|| format!("schema path not found: {schema_path}:{path}"))?;
    }
    let arr = current
        .as_array()
        .ok_or_else(|| format!("schema enum path is not array: {schema_path}:{path}"))?;
    Ok(arr
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect())
}

pub fn verify_audit_chain(path: &str) -> Result<String, String> {
    verify_audit_chain_with_mirror(path, None)
}

pub fn verify_audit_chain_with_mirror(
    path: &str,
    mirror_path: Option<&str>,
) -> Result<String, String> {
    let records = parse_and_verify_audit_chain(path)?;
    let count = records.len();

    if let Some(mirror) = mirror_path {
        let mirror_records = parse_and_verify_audit_chain(mirror)?;
        if records.len() != mirror_records.len() {
            return Err(format!(
                "mirror divergence: record count differs (primary={}, mirror={})",
                records.len(),
                mirror_records.len()
            ));
        }
        for (idx, (lhs, rhs)) in records.iter().zip(mirror_records.iter()).enumerate() {
            if lhs.record_hash != rhs.record_hash {
                return Err(format!(
                    "mirror divergence at line {}: primary_hash={} mirror_hash={}",
                    idx + 1,
                    lhs.record_hash,
                    rhs.record_hash
                ));
            }
        }
        return Ok(format!(
            "audit chain verified: {count} records (mirror matched: {})",
            mirror
        ));
    }

    Ok(format!("audit chain verified: {count} records"))
}

fn parse_and_verify_audit_chain(path: &str) -> Result<Vec<AuditRecord>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut prev: Option<String> = None;
    let mut records = Vec::new();

    for (idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let rec: AuditRecord = serde_json::from_str(line)
            .map_err(|e| format!("line {} parse failed: {e}", idx + 1))?;
        if idx > 0 && rec.prev_hash != prev {
            return Err(format!(
                "line {} prev_hash mismatch: expected {:?}, got {:?}",
                idx + 1,
                prev,
                rec.prev_hash
            ));
        }
        let mut seeded = rec.clone();
        seeded.record_hash.clear();
        let seed = serde_json::to_string(&seeded)
            .map_err(|e| format!("line {} hash seed serialize failed: {e}", idx + 1))?;
        let expected_hash = hash_hex(seed.as_bytes());
        if rec.record_hash != expected_hash {
            return Err(format!(
                "line {} record_hash mismatch: expected {}, got {}",
                idx + 1,
                expected_hash,
                rec.record_hash
            ));
        }
        prev = Some(rec.record_hash.clone());
        records.push(rec);
    }

    Ok(records)
}

fn validate_event(e: &Event) -> Result<(), String> {
    if e.v != CONTRACT_VERSION {
        return Err("v must be 1".to_string());
    }
    if e.event_id.is_empty()
        || e.tenant_id.is_empty()
        || e.source.is_empty()
        || e.room_id.is_empty()
        || e.actor.id.is_empty()
    {
        return Err("missing required field".to_string());
    }
    match e.actor.actor_type.as_str() {
        "human" | "service" | "system" => {}
        _ => return Err("invalid actor.type".to_string()),
    }
    if e.content.content_type != "text" {
        return Err("content.type must be text".to_string());
    }
    if parse_event_ts(&e.ts).is_none() {
        return Err("ts must be RFC3339".to_string());
    }
    Ok(())
}

fn validate_response_plan(p: &ResponsePlan) -> Result<(), String> {
    if p.v != CONTRACT_VERSION {
        return Err("response_plan.v must be 1".to_string());
    }
    if p.plan_id.is_empty() || p.tenant_id.is_empty() || p.actions.is_empty() {
        return Err("invalid response plan".to_string());
    }
    Ok(())
}

fn action_name(a: &Action) -> &'static str {
    match a.action_type {
        ActionType::DoNothing => "do_nothing",
        ActionType::RequestGeneration => "request_generation",
        ActionType::SendMessage => "send_message",
        ActionType::SendReply => "send_reply",
        ActionType::StartAgentJob => "start_agent_job",
        ActionType::RequestApproval => "request_approval",
    }
}

fn requested_action_mode(event: &Event) -> &str {
    event
        .extensions
        .get("arbiter_action")
        .and_then(|v| v.as_str())
        .unwrap_or("request_generation")
}

fn intent_name(intent: Intent) -> &'static str {
    match intent {
        Intent::Ignore => "IGNORE",
        Intent::Reply => "REPLY",
        Intent::Message => "MESSAGE",
    }
}

fn event_key(tenant_id: &str, event_id: &str) -> String {
    format!("{tenant_id}:{event_id}")
}

fn room_key(tenant_id: &str, room_id: &str) -> String {
    format!("{tenant_id}:{room_id}")
}

fn pending_key(tenant_id: &str, action_id: &str) -> String {
    format!("{tenant_id}:{action_id}")
}

fn action_index_key(tenant_id: &str, action_id: &str) -> String {
    format!("{tenant_id}:{action_id}")
}

fn action_result_store_key(tenant_id: &str, plan_id: &str, action_id: &str) -> String {
    format!("{tenant_id}:{plan_id}:{action_id}")
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn canonical_json_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(v) => v.to_string(),
        Value::Number(v) => v.to_string(),
        Value::String(v) => serde_json::to_string(v).unwrap_or_else(|_| "\"\"".to_string()),
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(canonical_json_string).collect();
            format!("[{}]", parts.join(","))
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_unstable();
            let parts: Vec<String> = keys
                .into_iter()
                .map(|k| {
                    let key = serde_json::to_string(k).unwrap_or_else(|_| "\"\"".to_string());
                    let val = canonical_json_string(&map[k]);
                    format!("{key}:{val}")
                })
                .collect();
            format!("{{{}}}", parts.join(","))
        }
    }
}

fn is_valid_job_transition(current: Option<&str>, next: &str) -> bool {
    match current {
        None => true,
        Some("completed" | "failed" | "cancelled") => current == Some(next),
        Some("started") => matches!(
            next,
            "started" | "heartbeat" | "completed" | "failed" | "cancelled"
        ),
        Some("heartbeat") => matches!(next, "heartbeat" | "completed" | "failed" | "cancelled"),
        Some(_) => false,
    }
}

fn is_valid_approval_transition(current: Option<&str>, next: &str) -> bool {
    match current {
        None => true,
        Some("approved" | "rejected" | "expired") => current == Some(next),
        Some("requested") => matches!(next, "requested" | "approved" | "rejected" | "expired"),
        Some(_) => false,
    }
}

fn api_error(error: String) -> (StatusCode, Json<Value>) {
    if let Some(message) = error.strip_prefix("conflict.payload_mismatch:") {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error": {"code":"conflict.payload_mismatch","message": message.trim()}})),
        );
    }
    if let Some(message) = error.strip_prefix("conflict.invalid_transition:") {
        return (
            StatusCode::CONFLICT,
            Json(
                json!({"error": {"code":"conflict.invalid_transition","message": message.trim()}}),
            ),
        );
    }
    if let Some(message) = error.strip_prefix("internal.audit_write_failed:") {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                json!({"error": {"code":"internal.audit_write_failed","message": message.trim()}}),
            ),
        );
    }
    validation_error_response(error)
}

fn validation_error_response(message: String) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error": {"code":"validation_error","message": message}})),
    )
}

fn internal_error_response(message: String) -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": {"code":"internal_error","message": message}})),
    )
}

fn action_result_error_from_value(value: Value) -> arbiter_contracts::ActionResultError {
    serde_json::from_value(value).unwrap_or(arbiter_contracts::ActionResultError {
        code: Some("invalid_error_payload".to_string()),
        message: Some("stored action-result error payload is invalid".to_string()),
        details: Map::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_response_plan_rejects_empty_actions() {
        let p = ResponsePlan {
            v: CONTRACT_VERSION,
            plan_id: "p".to_string(),
            tenant_id: "t".to_string(),
            room_id: "r".to_string(),
            actions: vec![],
            policy_decisions: vec![],
            debug: Map::new(),
        };
        assert!(validate_response_plan(&p).is_err());
    }
}
