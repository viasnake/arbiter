use arbiter_config::{Audit, Authz, AuthzCache, Config, Gate, Planner, Server, Store};
use arbiter_server::{build_app, verify_audit_chain};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use jsonschema::Validator;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tower::util::ServiceExt;

fn test_config() -> Config {
    test_config_with_authz_audit(true)
}

fn test_config_with_authz_audit(include_authz_decision: bool) -> Config {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    let audit_path = std::env::temp_dir().join(format!("arbiter-audit-{nanos}.jsonl"));
    Config {
        server: Server {
            listen_addr: "127.0.0.1:0".to_string(),
        },
        store: Store {
            kind: "memory".to_string(),
            sqlite_path: None,
        },
        authz: Authz {
            mode: "builtin".to_string(),
            endpoint: None,
            timeout_ms: 100,
            fail_mode: "deny".to_string(),
            retry_max_attempts: 2,
            retry_backoff_ms: 0,
            circuit_breaker_failures: 3,
            circuit_breaker_open_ms: 3000,
            cache: AuthzCache {
                enabled: true,
                ttl_ms: 30000,
                max_entries: 100,
            },
        },
        gate: Gate {
            cooldown_ms: 3000,
            max_queue: 10,
            tenant_rate_limit_per_min: 0,
        },
        planner: Planner {
            reply_policy: "all".to_string(),
            reply_probability: 0.0,
            approval_timeout_ms: 900000,
            approval_escalation_on_expired: true,
        },
        audit: Audit {
            sink: "jsonl".to_string(),
            jsonl_path: audit_path.to_string_lossy().to_string(),
            include_authz_decision,
            immutable_mirror_path: None,
        },
    }
}

fn test_config_sqlite(db_path: &str) -> Config {
    let mut cfg = test_config();
    cfg.store.kind = "sqlite".to_string();
    cfg.store.sqlite_path = Some(db_path.to_string());
    cfg
}

async fn spawn_mock_authz_invalid_policy_version() -> String {
    async fn handler() -> Json<Value> {
        Json(json!({
            "v": 1,
            "decision": "allow",
            "reason_code": "ok",
            "policy_version": "",
            "obligations": {},
            "ttl_ms": 1000
        }))
    }

    let app = Router::new().route("/v1/authorize", post(handler));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}/v1/authorize")
}

async fn spawn_mock_authz_flaky_then_allow() -> String {
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_clone = Arc::clone(&calls);
    async fn ok_decision() -> Json<Value> {
        Json(json!({
            "v": 1,
            "decision": "allow",
            "reason_code": "ok",
            "policy_version": "policy:v1",
            "obligations": {},
            "ttl_ms": 1000
        }))
    }

    let app = Router::new().route(
        "/v1/authorize",
        post(move || {
            let calls = Arc::clone(&calls_clone);
            async move {
                let n = calls.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({"error":"temp"})),
                    )
                } else {
                    (StatusCode::OK, ok_decision().await)
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}/v1/authorize")
}

async fn spawn_mock_authz_always_500(counter: Arc<AtomicUsize>) -> String {
    let app = Router::new().route(
        "/v1/authorize",
        post(move || {
            let c = Arc::clone(&counter);
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error":"down"})),
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}/v1/authorize")
}

fn sample_event(event_id: &str) -> Value {
    json!({
        "v": 1,
        "event_id": event_id,
        "tenant_id": "tenant-a",
        "source": "slack",
        "room_id": "room-1",
        "actor": {
            "type": "human",
            "id": "user-1",
            "roles": ["member"],
            "claims": {}
        },
        "content": {
            "type": "text",
            "text": "hello @arbiter",
            "reply_to": null
        },
        "ts": "2026-02-13T00:00:00Z",
        "extensions": {}
    })
}

#[tokio::test]
async fn healthz_ok() {
    let app = build_app(test_config()).await.unwrap();
    let res = app
        .oneshot(
            Request::builder()
                .uri("/v1/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn idempotency_same_event_same_plan() {
    let app = build_app(test_config()).await.unwrap();
    let event = sample_event("evt-1");

    let req1 = Request::builder()
        .method("POST")
        .uri("/v1/events")
        .header("content-type", "application/json")
        .body(Body::from(event.to_string()))
        .unwrap();
    let res1 = app.clone().oneshot(req1).await.unwrap();
    assert_eq!(res1.status(), StatusCode::OK);
    let body1 = axum::body::to_bytes(res1.into_body(), usize::MAX)
        .await
        .unwrap();
    let p1: Value = serde_json::from_slice(&body1).unwrap();

    let req2 = Request::builder()
        .method("POST")
        .uri("/v1/events")
        .header("content-type", "application/json")
        .body(Body::from(event.to_string()))
        .unwrap();
    let res2 = app.oneshot(req2).await.unwrap();
    assert_eq!(res2.status(), StatusCode::OK);
    let body2 = axum::body::to_bytes(res2.into_body(), usize::MAX)
        .await
        .unwrap();
    let p2: Value = serde_json::from_slice(&body2).unwrap();

    assert_eq!(p1, p2);
}

#[test]
fn event_input_and_plan_output_match_schemas() {
    let event_schema_text =
        std::fs::read_to_string(repo_path("contracts/v1/event.schema.json")).unwrap();
    let event_schema: Value = serde_json::from_str(&event_schema_text).unwrap();
    let event_validator: Validator = jsonschema::validator_for(&event_schema).unwrap();

    let plan_schema_text =
        std::fs::read_to_string(repo_path("contracts/v1/response_plan.schema.json")).unwrap();
    let mut plan_schema: Value = serde_json::from_str(&plan_schema_text).unwrap();
    let action_schema_text =
        std::fs::read_to_string(repo_path("contracts/v1/action.schema.json")).unwrap();
    let action_schema: Value = serde_json::from_str(&action_schema_text).unwrap();
    plan_schema["properties"]["actions"]["items"]["$ref"] =
        Value::String("#/$defs/action".to_string());
    plan_schema["$defs"] = json!({"action": action_schema});
    let plan_validator: Validator = jsonschema::validator_for(&plan_schema).unwrap();

    let event = sample_event("evt-schema");
    assert!(event_validator.validate(&event).is_ok());

    let rt = tokio::runtime::Runtime::new().unwrap();
    let body = rt.block_on(async {
        let app = build_app(test_config()).await.unwrap();
        let req = Request::builder()
            .method("POST")
            .uri("/v1/events")
            .header("content-type", "application/json")
            .body(Body::from(event.to_string()))
            .unwrap();

        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap()
    });
    let plan: Value = serde_json::from_slice(&body).unwrap();
    assert!(plan_validator.validate(&plan).is_ok());
}

#[tokio::test]
async fn golden_determinism_vectors_if_present() {
    let root_path = repo_path("test-vectors/determinism");
    let root = root_path.as_path();
    if !root.exists() {
        return;
    }

    let app = build_app(test_config()).await.unwrap();
    for entry in std::fs::read_dir(root).unwrap() {
        let case_dir = entry.unwrap().path();
        if !case_dir.is_dir() {
            continue;
        }

        let event_path = case_dir.join("event.json");
        let expected_path_candidates = [
            case_dir.join("response_plan.json"),
            case_dir.join("expected_response_plan.json"),
            case_dir.join("plan.json"),
        ];
        if !event_path.exists() {
            continue;
        }
        let expected_path = expected_path_candidates.into_iter().find(|p| p.exists());
        if expected_path.is_none() {
            continue;
        }
        let expected_text = std::fs::read_to_string(expected_path.unwrap()).unwrap();
        let expected: Value = serde_json::from_str(&expected_text).unwrap();
        let event_text = std::fs::read_to_string(event_path).unwrap();

        let req = Request::builder()
            .method("POST")
            .uri("/v1/events")
            .header("content-type", "application/json")
            .body(Body::from(event_text))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let actual: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(actual, expected);
    }
}

fn repo_path(relative: &str) -> PathBuf {
    let mut base = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    base.push("../..");
    base.push(relative);
    base
}

#[tokio::test]
async fn audit_trace_includes_authz_when_enabled() {
    let cfg = test_config_with_authz_audit(true);
    let audit_path = cfg.audit.jsonl_path.clone();
    let app = build_app(cfg).await.unwrap();
    let event = sample_event("evt-audit-enabled");

    let req = Request::builder()
        .method("POST")
        .uri("/v1/events")
        .header("content-type", "application/json")
        .body(Body::from(event.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let audit_text = std::fs::read_to_string(audit_path).unwrap();
    let last_line = audit_text.lines().last().unwrap();
    let rec: Value = serde_json::from_str(last_line).unwrap();
    assert!(rec["decision_trace"]["authz"].is_object());
    assert_eq!(rec["decision_trace"]["authz"]["result"], "allow");
    assert!(rec["decision_trace"]["planner"]["seed"].is_number());
}

#[tokio::test]
async fn audit_trace_omits_authz_when_disabled() {
    let cfg = test_config_with_authz_audit(false);
    let audit_path = cfg.audit.jsonl_path.clone();
    let app = build_app(cfg).await.unwrap();
    let event = sample_event("evt-audit-disabled");

    let req = Request::builder()
        .method("POST")
        .uri("/v1/events")
        .header("content-type", "application/json")
        .body(Body::from(event.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let audit_text = std::fs::read_to_string(audit_path).unwrap();
    let last_line = audit_text.lines().last().unwrap();
    let rec: Value = serde_json::from_str(last_line).unwrap();
    assert!(rec["decision_trace"]["authz"].is_null());
}

#[tokio::test]
async fn cooldown_uses_server_time_even_when_event_ts_is_future_or_past() {
    let mut cfg = test_config();
    cfg.gate.cooldown_ms = 60_000;
    let app = build_app(cfg).await.unwrap();

    let first_event = sample_event("evt-cooldown-1");
    let req1 = Request::builder()
        .method("POST")
        .uri("/v1/events")
        .header("content-type", "application/json")
        .body(Body::from(first_event.to_string()))
        .unwrap();
    let res1 = app.clone().oneshot(req1).await.unwrap();
    assert_eq!(res1.status(), StatusCode::OK);
    let body1 = axum::body::to_bytes(res1.into_body(), usize::MAX)
        .await
        .unwrap();
    let plan1: Value = serde_json::from_slice(&body1).unwrap();
    let action_id = plan1["actions"][0]["action_id"].as_str().unwrap();
    let plan_id = plan1["plan_id"].as_str().unwrap();

    let generation = json!({
        "v": 1,
        "plan_id": plan_id,
        "action_id": action_id,
        "tenant_id": "tenant-a",
        "text": "generated"
    });
    let gen_req = Request::builder()
        .method("POST")
        .uri("/v1/generations")
        .header("content-type", "application/json")
        .body(Body::from(generation.to_string()))
        .unwrap();
    let gen_res = app.clone().oneshot(gen_req).await.unwrap();
    assert_eq!(gen_res.status(), StatusCode::OK);

    for (event_id, ts) in [
        ("evt-cooldown-future", "2099-01-01T00:00:00Z"),
        ("evt-cooldown-past", "2000-01-01T00:00:00Z"),
    ] {
        let mut event = sample_event(event_id);
        event["ts"] = Value::String(ts.to_string());
        let req = Request::builder()
            .method("POST")
            .uri("/v1/events")
            .header("content-type", "application/json")
            .body(Body::from(event.to_string()))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let plan: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(plan["actions"][0]["type"], "do_nothing");
        assert_eq!(
            plan["actions"][0]["payload"]["reason_code"],
            "gate_cooldown"
        );
    }
}

#[tokio::test]
async fn sqlite_store_keeps_idempotency_across_app_restart() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    let db_path = std::env::temp_dir().join(format!("arbiter-store-{nanos}.db"));
    let db_path_str = db_path.to_string_lossy().to_string();

    let event = sample_event("evt-sqlite-persist");

    let app1 = build_app(test_config_sqlite(&db_path_str)).await.unwrap();
    let req1 = Request::builder()
        .method("POST")
        .uri("/v1/events")
        .header("content-type", "application/json")
        .body(Body::from(event.to_string()))
        .unwrap();
    let res1 = app1.oneshot(req1).await.unwrap();
    assert_eq!(res1.status(), StatusCode::OK);
    let body1 = axum::body::to_bytes(res1.into_body(), usize::MAX)
        .await
        .unwrap();
    let p1: Value = serde_json::from_slice(&body1).unwrap();

    let app2 = build_app(test_config_sqlite(&db_path_str)).await.unwrap();
    let req2 = Request::builder()
        .method("POST")
        .uri("/v1/events")
        .header("content-type", "application/json")
        .body(Body::from(event.to_string()))
        .unwrap();
    let res2 = app2.oneshot(req2).await.unwrap();
    assert_eq!(res2.status(), StatusCode::OK);
    let body2 = axum::body::to_bytes(res2.into_body(), usize::MAX)
        .await
        .unwrap();
    let p2: Value = serde_json::from_slice(&body2).unwrap();

    assert_eq!(p1, p2);
}

#[tokio::test]
async fn sqlite_store_persists_audit_records() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    let db_path = std::env::temp_dir().join(format!("arbiter-store-audit-{nanos}.db"));
    let db_path_str = db_path.to_string_lossy().to_string();

    let app = build_app(test_config_sqlite(&db_path_str)).await.unwrap();
    let event = sample_event("evt-sqlite-audit");
    let req = Request::builder()
        .method("POST")
        .uri("/v1/events")
        .header("content-type", "application/json")
        .body(Body::from(event.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let conn = Connection::open(db_path).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM audit_records", [], |row| row.get(0))
        .unwrap();
    assert!(count >= 1);
}

#[tokio::test]
async fn external_authz_invalid_contract_is_denied_in_fail_closed_mode() {
    let endpoint = spawn_mock_authz_invalid_policy_version().await;
    let mut cfg = test_config();
    cfg.authz.mode = "external_http".to_string();
    cfg.authz.endpoint = Some(endpoint);
    cfg.authz.fail_mode = "deny".to_string();
    cfg.authz.cache.enabled = false;

    let app = build_app(cfg).await.unwrap();
    let event = sample_event("evt-authz-invalid-contract");
    let req = Request::builder()
        .method("POST")
        .uri("/v1/events")
        .header("content-type", "application/json")
        .body(Body::from(event.to_string()))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let plan: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(plan["actions"][0]["type"], "do_nothing");
    assert_eq!(
        plan["actions"][0]["payload"]["reason_code"],
        "authz_contract_invalid_deny"
    );
}

#[tokio::test]
async fn external_authz_retries_and_recovers_on_second_attempt() {
    let endpoint = spawn_mock_authz_flaky_then_allow().await;
    let mut cfg = test_config();
    cfg.authz.mode = "external_http".to_string();
    cfg.authz.endpoint = Some(endpoint);
    cfg.authz.fail_mode = "deny".to_string();
    cfg.authz.cache.enabled = false;
    cfg.authz.retry_max_attempts = 2;
    cfg.authz.retry_backoff_ms = 0;

    let app = build_app(cfg).await.unwrap();
    let req = Request::builder()
        .method("POST")
        .uri("/v1/events")
        .header("content-type", "application/json")
        .body(Body::from(sample_event("evt-authz-retry").to_string()))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let plan: Value = serde_json::from_slice(&body).unwrap();
    assert_ne!(plan["actions"][0]["type"], "do_nothing");
}

#[tokio::test]
async fn external_authz_circuit_breaker_short_circuits_repeated_failures() {
    let calls = Arc::new(AtomicUsize::new(0));
    let endpoint = spawn_mock_authz_always_500(Arc::clone(&calls)).await;
    let mut cfg = test_config();
    cfg.authz.mode = "external_http".to_string();
    cfg.authz.endpoint = Some(endpoint);
    cfg.authz.fail_mode = "deny".to_string();
    cfg.authz.cache.enabled = false;
    cfg.authz.retry_max_attempts = 1;
    cfg.authz.circuit_breaker_failures = 1;
    cfg.authz.circuit_breaker_open_ms = 60_000;

    let app = build_app(cfg).await.unwrap();
    for event_id in ["evt-authz-cb-1", "evt-authz-cb-2"] {
        let req = Request::builder()
            .method("POST")
            .uri("/v1/events")
            .header("content-type", "application/json")
            .body(Body::from(sample_event(event_id).to_string()))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn can_choose_start_agent_job_action_via_extension() {
    let app = build_app(test_config()).await.unwrap();
    let mut event = sample_event("evt-job-mode");
    event["extensions"] = json!({"arbiter_action": "start_agent_job"});

    let req = Request::builder()
        .method("POST")
        .uri("/v1/events")
        .header("content-type", "application/json")
        .body(Body::from(event.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let plan: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(plan["actions"][0]["type"], "start_agent_job");

    let generation = json!({
        "v": 1,
        "plan_id": plan["plan_id"],
        "action_id": plan["actions"][0]["action_id"],
        "tenant_id": "tenant-a",
        "text": "should not execute"
    });
    let gen_req = Request::builder()
        .method("POST")
        .uri("/v1/generations")
        .header("content-type", "application/json")
        .body(Body::from(generation.to_string()))
        .unwrap();
    let gen_res = app.oneshot(gen_req).await.unwrap();
    let gen_body = axum::body::to_bytes(gen_res.into_body(), usize::MAX)
        .await
        .unwrap();
    let gen_plan: Value = serde_json::from_slice(&gen_body).unwrap();
    assert_eq!(
        gen_plan["actions"][0]["payload"]["reason_code"],
        "generation_unknown_action"
    );
}

#[tokio::test]
async fn can_choose_request_approval_action_via_extension() {
    let app = build_app(test_config()).await.unwrap();
    let mut event = sample_event("evt-approval-mode");
    event["extensions"] = json!({"arbiter_action": "request_approval"});

    let req = Request::builder()
        .method("POST")
        .uri("/v1/events")
        .header("content-type", "application/json")
        .body(Body::from(event.to_string()))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let plan: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(plan["actions"][0]["type"], "request_approval");
}

#[tokio::test]
async fn audit_jsonl_records_are_hash_chained() {
    let cfg = test_config_with_authz_audit(true);
    let audit_path = cfg.audit.jsonl_path.clone();
    let app = build_app(cfg).await.unwrap();

    for event_id in ["evt-chain-1", "evt-chain-2"] {
        let req = Request::builder()
            .method("POST")
            .uri("/v1/events")
            .header("content-type", "application/json")
            .body(Body::from(sample_event(event_id).to_string()))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let audit_text = std::fs::read_to_string(audit_path).unwrap();
    let lines: Vec<&str> = audit_text.lines().collect();
    assert!(lines.len() >= 2);

    let first: Value = serde_json::from_str(lines[lines.len() - 2]).unwrap();
    let second: Value = serde_json::from_str(lines[lines.len() - 1]).unwrap();

    let first_hash = first["record_hash"].as_str().unwrap();
    let second_prev = second["prev_hash"].as_str().unwrap();
    assert!(!first_hash.is_empty());
    assert_eq!(second_prev, first_hash);
}

#[tokio::test]
async fn job_status_event_is_idempotent() {
    let app = build_app(test_config()).await.unwrap();
    let payload = json!({
        "v": 1,
        "event_id": "job-status-evt-1",
        "tenant_id": "tenant-a",
        "job_id": "job-1",
        "status": "started",
        "ts": "2026-02-14T00:00:00Z"
    });

    let req1 = Request::builder()
        .method("POST")
        .uri("/v1/job-events")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();
    let res1 = app.clone().oneshot(req1).await.unwrap();
    let body1 = axum::body::to_bytes(res1.into_body(), usize::MAX)
        .await
        .unwrap();
    let p1: Value = serde_json::from_slice(&body1).unwrap();

    let req2 = Request::builder()
        .method("POST")
        .uri("/v1/job-events")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();
    let res2 = app.oneshot(req2).await.unwrap();
    let body2 = axum::body::to_bytes(res2.into_body(), usize::MAX)
        .await
        .unwrap();
    let p2: Value = serde_json::from_slice(&body2).unwrap();

    assert_eq!(p1, p2);
}

#[tokio::test]
async fn job_cancel_event_is_idempotent() {
    let app = build_app(test_config()).await.unwrap();
    let payload = json!({
        "v": 1,
        "event_id": "job-cancel-evt-1",
        "tenant_id": "tenant-a",
        "job_id": "job-1",
        "ts": "2026-02-14T00:00:00Z"
    });

    let req1 = Request::builder()
        .method("POST")
        .uri("/v1/job-cancel")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();
    let res1 = app.clone().oneshot(req1).await.unwrap();
    let body1 = axum::body::to_bytes(res1.into_body(), usize::MAX)
        .await
        .unwrap();
    let p1: Value = serde_json::from_slice(&body1).unwrap();

    let req2 = Request::builder()
        .method("POST")
        .uri("/v1/job-cancel")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();
    let res2 = app.oneshot(req2).await.unwrap();
    let body2 = axum::body::to_bytes(res2.into_body(), usize::MAX)
        .await
        .unwrap();
    let p2: Value = serde_json::from_slice(&body2).unwrap();

    assert_eq!(p1, p2);
}

#[tokio::test]
async fn request_approval_plan_contains_timeout_and_id() {
    let mut cfg = test_config();
    cfg.planner.approval_timeout_ms = 60_000;
    let app = build_app(cfg).await.unwrap();
    let mut event = sample_event("evt-approval-timeout");
    event["extensions"] = json!({"arbiter_action":"request_approval"});

    let req = Request::builder()
        .method("POST")
        .uri("/v1/events")
        .header("content-type", "application/json")
        .body(Body::from(event.to_string()))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let plan: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(plan["actions"][0]["type"], "request_approval");
    assert!(plan["actions"][0]["payload"]["approval_id"].is_string());
    assert!(plan["actions"][0]["payload"]["expires_at"].is_string());
}

#[tokio::test]
async fn approval_expired_event_sets_escalation_debug_field() {
    let mut cfg = test_config();
    cfg.planner.approval_escalation_on_expired = true;
    let app = build_app(cfg).await.unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/v1/approval-events")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "v": 1,
                "event_id": "approval-expired-1",
                "tenant_id": "tenant-a",
                "approval_id": "approval:1",
                "status": "expired",
                "ts": "2026-02-14T00:00:00Z"
            })
            .to_string(),
        ))
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let plan: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(plan["debug"]["escalation"], "notify_human");
}

#[tokio::test]
async fn immutable_mirror_sink_receives_audit_lines() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    let mirror = std::env::temp_dir().join(format!("arbiter-audit-mirror-{nanos}.jsonl"));
    let mut cfg = test_config();
    cfg.audit.immutable_mirror_path = Some(mirror.to_string_lossy().to_string());

    let app = build_app(cfg).await.unwrap();
    let req = Request::builder()
        .method("POST")
        .uri("/v1/events")
        .header("content-type", "application/json")
        .body(Body::from(sample_event("evt-mirror").to_string()))
        .unwrap();
    let _ = app.oneshot(req).await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let content = std::fs::read_to_string(mirror).unwrap();
    assert!(!content.trim().is_empty());
}

#[tokio::test]
async fn audit_verify_tool_accepts_valid_chain() {
    let cfg = test_config_with_authz_audit(true);
    let audit_path = cfg.audit.jsonl_path.clone();
    let app = build_app(cfg).await.unwrap();

    for event_id in ["evt-verify-1", "evt-verify-2"] {
        let req = Request::builder()
            .method("POST")
            .uri("/v1/events")
            .header("content-type", "application/json")
            .body(Body::from(sample_event(event_id).to_string()))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let result = verify_audit_chain(&audit_path);
    assert!(result.is_ok());
}
