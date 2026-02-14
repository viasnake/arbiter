use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use arbiter_config::Config;
use arbiter_contracts::{
    Action, ActionType, AuthZDecision, AuthZReqData, AuthZRequest, AuthZResource, Event,
    GenerationResult, ResponsePlan, CONTRACT_VERSION,
};
use arbiter_kernel::{
    decide_intent, do_nothing_plan, evaluate_gate, minute_bucket, parse_event_ts,
    planner_probability, planner_seed, request_generation_plan, send_plan, GateConfig,
    GateDecision, Intent, PlannerConfig, RoomState,
};
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::sync::Mutex;

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
        .route("/v0/healthz", get(healthz))
        .route("/v0/events", post(events))
        .route("/v0/generations", post(generations))
        .route("/v0/action-results", post(action_results))
        .route("/v0/contracts", get(contracts))
        .with_state(state))
}

#[derive(Clone)]
struct AppState {
    cfg: Config,
    store: Arc<Mutex<MemoryStore>>,
    audit: Arc<AuditJsonl>,
    authz: Arc<AuthzEngine>,
}

impl AppState {
    async fn new(cfg: Config) -> Result<Self, String> {
        Ok(Self {
            authz: Arc::new(AuthzEngine::new(&cfg)?),
            audit: Arc::new(AuditJsonl::new(&cfg.audit.jsonl_path).await?),
            store: Arc::new(Mutex::new(MemoryStore::default())),
            cfg,
        })
    }

    async fn process_event(&self, event: Event) -> Result<ResponsePlan, String> {
        validate_event(&event)?;

        if let Some(existing) = {
            let store = self.store.lock().await;
            store
                .idempotency
                .get(&event_key(&event.tenant_id, &event.event_id))
                .cloned()
        } {
            self.audit
                .append(AuditRecord::new(
                    &event.tenant_id,
                    &event.event_id,
                    "process_event",
                    "idempotency_hit",
                    "idempotency_hit",
                    Some(existing.plan_id.clone()),
                ))
                .await;
            return Ok(existing);
        }

        parse_event_ts(&event.ts).ok_or_else(|| "invalid ts (RFC3339 required)".to_string())?;
        let server_now = Utc::now();

        let mut store = self.store.lock().await;
        let room_key = room_key(&event.tenant_id, &event.room_id);
        let room = store.rooms.get(&room_key).cloned().unwrap_or_default();
        let tenant_count = store
            .tenant_rate
            .get(&event.tenant_id)
            .and_then(|m| m.get(&minute_bucket(server_now)))
            .copied()
            .unwrap_or(0);

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
                .idempotency
                .insert(event_key(&event.tenant_id, &event.event_id), plan.clone());
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
                .await;
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
                .idempotency
                .insert(event_key(&event.tenant_id, &event.event_id), plan.clone());
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
                .await;
            return Ok(plan);
        }

        let planner_cfg = PlannerConfig {
            reply_policy: self.cfg.planner.reply_policy.clone(),
            reply_probability: self.cfg.planner.reply_probability,
        };
        let intent = decide_intent(&event, &planner_cfg);
        let planner_seed = planner_seed(&event.event_id);
        let sampled_probability = planner_probability(&event.event_id);

        let plan = match intent {
            Intent::Ignore => do_nothing_plan(
                &event.tenant_id,
                &event.room_id,
                &event.event_id,
                "planner_ignore",
            ),
            Intent::Reply | Intent::Message => {
                request_generation_plan(&event, intent, &authz.reason_code)
            }
        };
        validate_response_plan(&plan)?;

        let mut store = self.store.lock().await;
        if matches!(intent, Intent::Reply | Intent::Message) {
            let action = &plan.actions[0];
            let room_state = store.rooms.entry(room_key).or_default();
            room_state.generating = true;
            room_state.pending_queue_size += 1;

            store.pending.insert(
                pending_key(&event.tenant_id, &action.action_id),
                PendingGeneration {
                    tenant_id: event.tenant_id.clone(),
                    room_id: event.room_id.clone(),
                    action_id: action.action_id.clone(),
                    reply_to: event.content.reply_to.clone(),
                    intent,
                },
            );
        }

        store
            .tenant_rate
            .entry(event.tenant_id.clone())
            .or_default()
            .entry(minute_bucket(server_now))
            .and_modify(|v| *v += 1)
            .or_insert(1);

        store
            .idempotency
            .insert(event_key(&event.tenant_id, &event.event_id), plan.clone());
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
            .await;
        Ok(plan)
    }

    async fn process_generation(&self, input: GenerationResult) -> Result<ResponsePlan, String> {
        if input.v != CONTRACT_VERSION {
            return Err("v must be 0".to_string());
        }
        if input.tenant_id.is_empty() || input.plan_id.is_empty() || input.action_id.is_empty() {
            return Err("tenant_id, plan_id, action_id are required".to_string());
        }

        let mut store = self.store.lock().await;
        let key = pending_key(&input.tenant_id, &input.action_id);
        let pending = match store.pending.remove(&key) {
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
                    .await;
                return Ok(plan);
            }
        };

        let room_state = store
            .rooms
            .entry(room_key(&pending.tenant_id, &pending.room_id))
            .or_default();
        if room_state.pending_queue_size > 0 {
            room_state.pending_queue_size -= 1;
        }
        room_state.generating = room_state.pending_queue_size > 0;
        room_state.last_send_at = Some(Utc::now());
        drop(store);

        let plan = send_plan(
            &pending.tenant_id,
            &pending.room_id,
            &pending.action_id,
            &input.text,
            pending.reply_to.as_deref(),
        );
        validate_response_plan(&plan)?;

        self.audit
            .append(AuditRecord::new(
                &input.tenant_id,
                &input.action_id,
                "generation_result",
                "ok",
                action_name(&plan.actions[0]),
                Some(plan.plan_id.clone()),
            ))
            .await;
        Ok(plan)
    }
}

async fn healthz() -> (StatusCode, &'static str) {
    (StatusCode::OK, "ok")
}

async fn contracts() -> Json<Value> {
    Json(json!({
        "version": "0.0.1",
        "actions": {
            "enabled": ["do_nothing", "request_generation", "send_message", "send_reply"],
            "reserved": ["start_agent_job", "request_approval"]
        }
    }))
}

async fn events(
    State(state): State<AppState>,
    Json(event): Json<Event>,
) -> Result<Json<ResponsePlan>, (StatusCode, Json<Value>)> {
    state.process_event(event).await.map(Json).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"code":"validation_error","message": e}})),
        )
    })
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

#[derive(Debug, Deserialize)]
struct ActionResultInput {
    tenant_id: String,
    correlation_id: String,
    #[allow(dead_code)]
    reason_code: Option<String>,
}

async fn action_results(
    State(state): State<AppState>,
    Json(input): Json<ActionResultInput>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    if input.tenant_id.is_empty() || input.correlation_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(
                json!({"error": {"code":"validation_error","message":"tenant_id and correlation_id are required"}}),
            ),
        ));
    }
    state
        .audit
        .append(AuditRecord::new(
            &input.tenant_id,
            &input.correlation_id,
            "action_result",
            "recorded",
            "action_result",
            None,
        ))
        .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Default)]
struct MemoryStore {
    idempotency: HashMap<String, ResponsePlan>,
    rooms: HashMap<String, RoomState>,
    pending: HashMap<String, PendingGeneration>,
    tenant_rate: HashMap<String, HashMap<i64, usize>>,
}

struct PendingGeneration {
    tenant_id: String,
    room_id: String,
    action_id: String,
    reply_to: Option<String>,
    #[allow(dead_code)]
    intent: Intent,
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
    cache_enabled: bool,
    cache_ttl: Duration,
    cache_max_entries: usize,
    cache: Arc<Mutex<HashMap<String, CachedDecision>>>,
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
            cache_enabled: cfg.authz.cache.enabled,
            cache_ttl: Duration::from_millis(cfg.authz.cache.ttl_ms as u64),
            cache_max_entries: cfg.authz.cache.max_entries,
            cache: Arc::new(Mutex::new(HashMap::new())),
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
            _ => return self.on_failure(),
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

        let response = match self.client.post(endpoint).json(&request).send().await {
            Ok(v) => v,
            Err(_) => return self.on_failure(),
        };
        if !response.status().is_success() {
            return self.on_failure();
        }

        let decision: AuthZDecision = match response.json().await {
            Ok(v) => v,
            Err(_) => return self.on_failure(),
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

    fn on_failure(&self) -> AuthzOutcome {
        match self.fail_mode.as_str() {
            "allow" => AuthzOutcome {
                allow: true,
                reason_code: "authz_error_allow".to_string(),
                policy_version: None,
            },
            "fallback_builtin" => AuthzOutcome {
                allow: true,
                reason_code: "authz_error_fallback_builtin".to_string(),
                policy_version: Some("builtin:fallback".to_string()),
            },
            _ => AuthzOutcome {
                allow: false,
                reason_code: "authz_error_deny".to_string(),
                policy_version: None,
            },
        }
    }
}

struct AuditJsonl {
    file: Arc<Mutex<tokio::fs::File>>,
}

#[derive(Serialize)]
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
}

#[derive(Serialize)]
struct DecisionTrace {
    #[serde(skip_serializing_if = "Option::is_none")]
    gate: Option<StageDecision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    authz: Option<AuthzDecisionTrace>,
    #[serde(skip_serializing_if = "Option::is_none")]
    planner: Option<PlannerDecisionTrace>,
}

#[derive(Serialize)]
struct StageDecision {
    result: String,
    reason_code: String,
}

#[derive(Serialize)]
struct AuthzDecisionTrace {
    result: String,
    reason_code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    policy_version: Option<String>,
}

#[derive(Serialize)]
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
        }
    }

    fn with_trace(mut self, trace: DecisionTrace) -> Self {
        self.decision_trace = Some(trace);
        self
    }
}

impl AuditJsonl {
    async fn new(path: &str) -> Result<Self, String> {
        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await
            .map_err(|e| e.to_string())?;
        Ok(Self {
            file: Arc::new(Mutex::new(file)),
        })
    }

    async fn append(&self, rec: AuditRecord) {
        let mut file = self.file.lock().await;
        if let Ok(line) = serde_json::to_string(&rec) {
            use tokio::io::AsyncWriteExt;
            let _ = file.write_all(line.as_bytes()).await;
            let _ = file.write_all(b"\n").await;
        }
    }
}

fn validate_event(e: &Event) -> Result<(), String> {
    if e.v != CONTRACT_VERSION {
        return Err("v must be 0".to_string());
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
        return Err("response_plan.v must be 0".to_string());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_response_plan_rejects_empty_actions() {
        let p = ResponsePlan {
            v: 0,
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
