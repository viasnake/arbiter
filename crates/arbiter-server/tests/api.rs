use arbiter_config::{Approver, Audit, Config, Governance, Policy, Server, Store};
use arbiter_contracts::{DecisionEffect, RunStatus, StepStatus, API_VERSION};
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
            capability_allowlist: vec![],
            capability_denylist: vec![],
            permit_ttl_seconds: 300,
            idempotency_retention_hours: 24,
        },
        policy: Policy {
            version: "policy:test".to_string(),
            require_approval_for_write_external: true,
            require_approval_for_notify: false,
            require_approval_for_start_job: false,
            require_approval_for_production: true,
        },
        approver: Approver {
            default_approvers: vec!["team-lead".to_string()],
            production_approvers: vec!["prod-owner".to_string()],
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

fn sqlite_test_config() -> Config {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    Config {
        server: Server {
            listen_addr: "127.0.0.1:0".to_string(),
        },
        store: Store {
            kind: "sqlite".to_string(),
            sqlite_path: Some(
                std::env::temp_dir()
                    .join(format!("arbiter-{nanos}.db"))
                    .to_string_lossy()
                    .to_string(),
            ),
        },
        governance: Governance {
            allowed_providers: vec!["generic".to_string()],
            capability_allowlist: vec![],
            capability_denylist: vec![],
            permit_ttl_seconds: 300,
            idempotency_retention_hours: 24,
        },
        policy: Policy {
            version: "policy:test".to_string(),
            require_approval_for_write_external: true,
            require_approval_for_notify: false,
            require_approval_for_start_job: false,
            require_approval_for_production: true,
        },
        approver: Approver {
            default_approvers: vec!["team-lead".to_string()],
            production_approvers: vec!["prod-owner".to_string()],
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
        "objective": "deploy service",
        "environment_hint": "prod",
        "metadata": {"service": "billing"}
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
                .uri("/v1/operation-requests")
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
    let run_id = payload["run_id"].as_str().unwrap().to_string();

    let fetched = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/runs/{run_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(fetched.status(), StatusCode::OK);
}

#[tokio::test]
async fn same_request_id_same_payload_is_idempotent() {
    let app = build_app(test_config()).await.unwrap();
    let payload = sample_request("req-idem");

    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/operation-requests")
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::CREATED);

    let second = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/operation-requests")
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn same_request_id_different_payload_conflicts() {
    let app = build_app(test_config()).await.unwrap();
    let first = sample_request("req-conflict");
    let second = json!({
        "request_id": "req-conflict",
        "source": "api",
        "requester": "alice",
        "objective": "different objective",
        "environment_hint": "prod",
        "metadata": {"service": "orders"}
    });

    let res1 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/operation-requests")
                .header("content-type", "application/json")
                .body(Body::from(first.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res1.status(), StatusCode::CREATED);

    let res2 = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/operation-requests")
                .header("content-type", "application/json")
                .body(Body::from(second.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res2.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn approval_required_grant_and_result_success() {
    let app = build_app(test_config()).await.unwrap();

    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/operation-requests")
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
    let run_id = created_json["run_id"].as_str().unwrap();

    let intent = json!({
        "client_step_id": "step-a",
        "intent_type": "change",
        "capability": "write_db",
        "target": "database.main",
        "risk_level": "write",
        "provider": "generic",
        "metadata": {"table": "users"}
    });

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/runs/{run_id}/step-intents"))
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
    assert_eq!(step["status"], json!(StepStatus::ApprovalRequired));
    let approval_id = step["approval_id"].as_str().unwrap();
    let step_id = step["step_id"].as_str().unwrap();

    let grant = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/approvals/{approval_id}/grant"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"actor": "approver1", "reason": "approved"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(grant.status(), StatusCode::OK);

    let result = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/runs/{run_id}/step-results"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "step_id": step_id,
                        "execution_result": "ok",
                        "artifacts": {},
                        "error": null,
                        "executor_metadata": {"executor": "agent"}
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(result.status(), StatusCode::OK);
    let result_body = axum::body::to_bytes(result.into_body(), usize::MAX)
        .await
        .unwrap();
    let result_json: Value = serde_json::from_slice(&result_body).unwrap();
    assert_eq!(result_json["run_status"], json!(RunStatus::Succeeded));
}

#[tokio::test]
async fn approval_deny_blocks_run() {
    let app = build_app(test_config()).await.unwrap();
    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/operation-requests")
                .header("content-type", "application/json")
                .body(Body::from(sample_request("req-deny").to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let created_body = axum::body::to_bytes(created.into_body(), usize::MAX)
        .await
        .unwrap();
    let created_json: Value = serde_json::from_slice(&created_body).unwrap();
    let run_id = created_json["run_id"].as_str().unwrap();

    let intent = json!({
        "client_step_id": "step-deny",
        "intent_type": "change",
        "capability": "write_db",
        "target": "database.main",
        "risk_level": "write",
        "provider": "generic",
        "metadata": {}
    });
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/runs/{run_id}/step-intents"))
                .header("content-type", "application/json")
                .body(Body::from(intent.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let step: Value = serde_json::from_slice(&body).unwrap();
    let approval_id = step["approval_id"].as_str().unwrap();

    let deny = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/approvals/{approval_id}/deny"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"actor": "approver1"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deny.status(), StatusCode::OK);

    let fetched = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/runs/{run_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let fetched_body = axum::body::to_bytes(fetched.into_body(), usize::MAX)
        .await
        .unwrap();
    let fetched_json: Value = serde_json::from_slice(&fetched_body).unwrap();
    assert_eq!(fetched_json["run"]["status"], json!(RunStatus::Blocked));
}

#[tokio::test]
async fn audit_endpoint_returns_events() {
    let app = build_app(test_config()).await.unwrap();
    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/operation-requests")
                .header("content-type", "application/json")
                .body(Body::from(sample_request("req-audit").to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let created_body = axum::body::to_bytes(created.into_body(), usize::MAX)
        .await
        .unwrap();
    let created_json: Value = serde_json::from_slice(&created_body).unwrap();
    let run_id = created_json["run_id"].as_str().unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/audit/runs/{run_id}"))
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
    assert!(!payload["events"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn v2_routes_are_not_available() {
    let app = build_app(test_config()).await.unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v2/operation-requests")
                .header("content-type", "application/json")
                .body(Body::from(sample_request("req-old-v2").to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn sqlite_backend_persists_runs_across_restart() {
    let cfg = sqlite_test_config();
    let app1 = build_app(cfg.clone()).await.unwrap();
    let created = app1
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/operation-requests")
                .header("content-type", "application/json")
                .body(Body::from(sample_request("req-sqlite").to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let created_body = axum::body::to_bytes(created.into_body(), usize::MAX)
        .await
        .unwrap();
    let created_json: Value = serde_json::from_slice(&created_body).unwrap();
    let run_id = created_json["run_id"].as_str().unwrap().to_string();

    let app2 = build_app(cfg).await.unwrap();
    let fetched = app2
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/runs/{run_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(fetched.status(), StatusCode::OK);
}

#[tokio::test]
async fn audit_chain_continues_after_restart() {
    let cfg = sqlite_test_config();
    let audit_path = cfg.audit.jsonl_path.clone();

    let app1 = build_app(cfg.clone()).await.unwrap();
    let res1 = app1
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/operation-requests")
                .header("content-type", "application/json")
                .body(Body::from(sample_request("req-restart-1").to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res1.status(), StatusCode::CREATED);

    let app2 = build_app(cfg).await.unwrap();
    let res2 = app2
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/operation-requests")
                .header("content-type", "application/json")
                .body(Body::from(sample_request("req-restart-2").to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res2.status(), StatusCode::CREATED);

    let result = arbiter_server::verify_audit_chain(&audit_path);
    assert!(result.is_ok());
}

#[tokio::test]
async fn step_result_before_approval_returns_423() {
    let app = build_app(test_config()).await.unwrap();
    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/operation-requests")
                .header("content-type", "application/json")
                .body(Body::from(sample_request("req-locked").to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let created_body = axum::body::to_bytes(created.into_body(), usize::MAX)
        .await
        .unwrap();
    let created_json: Value = serde_json::from_slice(&created_body).unwrap();
    let run_id = created_json["run_id"].as_str().unwrap();

    let intent = json!({
        "client_step_id": "step-locked",
        "intent_type": "change",
        "capability": "write_db",
        "target": "database.main",
        "risk_level": "write",
        "provider": "generic",
        "metadata": {}
    });
    let step_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/runs/{run_id}/step-intents"))
                .header("content-type", "application/json")
                .body(Body::from(intent.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let step_body = axum::body::to_bytes(step_res.into_body(), usize::MAX)
        .await
        .unwrap();
    let step_json: Value = serde_json::from_slice(&step_body).unwrap();
    let step_id = step_json["step_id"].as_str().unwrap();

    let result = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/runs/{run_id}/step-results"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "step_id": step_id,
                        "execution_result": "ok",
                        "artifacts": {},
                        "error": null,
                        "executor_metadata": {}
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(result.status(), StatusCode::LOCKED);
}
