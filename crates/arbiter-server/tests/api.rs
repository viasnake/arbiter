use arbiter_config::{Audit, Authz, AuthzCache, Config, Gate, Planner, Server, Store};
use arbiter_server::build_app;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use jsonschema::Validator;
use serde_json::{json, Value};
use std::path::PathBuf;
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
        },
        audit: Audit {
            sink: "jsonl".to_string(),
            jsonl_path: audit_path.to_string_lossy().to_string(),
            include_authz_decision,
        },
    }
}

fn sample_event(event_id: &str) -> Value {
    json!({
        "v": 0,
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
                .uri("/v0/healthz")
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
        .uri("/v0/events")
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
        .uri("/v0/events")
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
        std::fs::read_to_string(repo_path("contracts/v0/event.schema.json")).unwrap();
    let event_schema: Value = serde_json::from_str(&event_schema_text).unwrap();
    let event_validator: Validator = jsonschema::validator_for(&event_schema).unwrap();

    let plan_schema_text =
        std::fs::read_to_string(repo_path("contracts/v0/response_plan.schema.json")).unwrap();
    let mut plan_schema: Value = serde_json::from_str(&plan_schema_text).unwrap();
    let action_schema_text =
        std::fs::read_to_string(repo_path("contracts/v0/action.schema.json")).unwrap();
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
            .uri("/v0/events")
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
            .uri("/v0/events")
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
        .uri("/v0/events")
        .header("content-type", "application/json")
        .body(Body::from(event.to_string()))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
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
        .uri("/v0/events")
        .header("content-type", "application/json")
        .body(Body::from(event.to_string()))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
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
        .uri("/v0/events")
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
        "v": 0,
        "plan_id": plan_id,
        "action_id": action_id,
        "tenant_id": "tenant-a",
        "text": "generated"
    });
    let gen_req = Request::builder()
        .method("POST")
        .uri("/v0/generations")
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
            .uri("/v0/events")
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
