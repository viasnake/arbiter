use arbiter_config::{Audit, Config, Governance, Policy, Server, Store};
use arbiter_contracts::{DecisionEffect, StepStatus, API_VERSION};
use arbiter_server::build_app;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};
use tower::util::ServiceExt;

fn test_config() -> Config {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    Config {
        server: Server {
            listen_addr: "127.0.0.1:0".to_string(),
        },
        store: Store {
            kind: "memory".to_string(),
            sqlite_path: None,
        },
        governance: Governance {
            allowed_providers: vec!["generic".to_string()],
        },
        policy: Policy {
            version: "policy:test".to_string(),
            require_approval_for_write_external: true,
            require_approval_for_notify: false,
            require_approval_for_start_job: false,
        },
        audit: Audit {
            jsonl_path: std::env::temp_dir()
                .join(format!("arbiter-audit-{nanos}.jsonl"))
                .to_string_lossy()
                .to_string(),
            immutable_mirror_path: None,
        },
    }
}

fn sample_request(request_id: &str) -> Value {
    json!({
        "request_id": request_id,
        "source": "api",
        "requester": "alice",
        "target_agent": "ops-agent",
        "objective": "deploy service",
        "payload": {"service": "billing"},
        "environment_hint": "prod",
        "correlation_id": "corr-1",
        "urgency": "high"
    })
}

#[tokio::test]
async fn healthz_ok() {
    let app = build_app(test_config()).await.unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn contracts_endpoint_ok() {
    let app = build_app(test_config()).await.unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/contracts")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["api_version"], API_VERSION);
}

#[tokio::test]
async fn create_run_and_fetch() {
    let app = build_app(test_config()).await.unwrap();

    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v2/operation-requests")
                .header("content-type", "application/json")
                .body(Body::from(sample_request("req-1").to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(created.into_body(), usize::MAX)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let run_id = payload["run"]["run_id"].as_str().unwrap().to_string();

    let fetched = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v2/runs/{run_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(fetched.status(), StatusCode::OK);
}

#[tokio::test]
async fn low_risk_step_gets_allow_and_permit() {
    let app = build_app(test_config()).await.unwrap();

    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v2/operation-requests")
                .header("content-type", "application/json")
                .body(Body::from(sample_request("req-2").to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let created_body = axum::body::to_bytes(created.into_body(), usize::MAX)
        .await
        .unwrap();
    let created_json: Value = serde_json::from_slice(&created_body).unwrap();
    let run_id = created_json["run"]["run_id"].as_str().unwrap();

    let intent = json!({
        "intent_id": "intent-1",
        "run_id": run_id,
        "step_type": "emit_output",
        "proposed_action": "return_summary",
        "risk_level": "low",
        "payload": {"text": "ok"}
    });

    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v2/runs/{run_id}/step-intents"))
                .header("content-type", "application/json")
                .body(Body::from(intent.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let step: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(step["decision"]["effect"], json!(DecisionEffect::Allow));
    assert_eq!(step["status"], json!(StepStatus::Permitted));
    assert!(step["permit"].is_object());
}

#[tokio::test]
async fn write_tool_step_requires_approval() {
    let app = build_app(test_config()).await.unwrap();

    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v2/operation-requests")
                .header("content-type", "application/json")
                .body(Body::from(sample_request("req-3").to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let created_body = axum::body::to_bytes(created.into_body(), usize::MAX)
        .await
        .unwrap();
    let created_json: Value = serde_json::from_slice(&created_body).unwrap();
    let run_id = created_json["run"]["run_id"].as_str().unwrap();

    let intent = json!({
        "intent_id": "intent-2",
        "run_id": run_id,
        "step_type": "tool_call",
        "proposed_action": "write_database",
        "risk_level": "write",
        "payload": {"table": "users"},
        "tool_name": "db_writer"
    });

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v2/runs/{run_id}/step-intents"))
                .header("content-type", "application/json")
                .body(Body::from(intent.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let step: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        step["decision"]["effect"],
        json!(DecisionEffect::RequireApproval)
    );
    assert_eq!(step["status"], json!(StepStatus::WaitingForApproval));
    let approval_id = step["approval_request"]["approval_id"].as_str().unwrap();

    let grant = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v2/approvals/{approval_id}/grant"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(grant.status(), StatusCode::NO_CONTENT);
}
