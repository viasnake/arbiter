use arbiter_config::{Audit, Config, Governance, Policy, Server, Store};
use arbiter_contracts::API_VERSION;
use arbiter_server::{build_app, verify_audit_chain, verify_audit_chain_with_mirror};
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
            allowed_providers: vec!["generic".to_string(), "email".to_string()],
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

fn test_config_sqlite(db_path: &str) -> Config {
    let mut cfg = test_config();
    cfg.store.kind = "sqlite".to_string();
    cfg.store.sqlite_path = Some(db_path.to_string());
    cfg
}

fn sample_event(event_id: &str) -> Value {
    json!({
        "tenant_id": "tenant-a",
        "event_id": event_id,
        "occurred_at": "2026-02-14T00:00:00Z",
        "source": "github",
        "kind": "webhook_received",
        "subject": "issue/1",
        "summary": "new issue arrived",
        "payload_ref": "s3://bucket/raw/1.json",
        "labels": {
            "provider": "generic",
            "action_type": "notify",
            "risk": "low",
            "operation": "emit_notification"
        },
        "context": {
            "repo": "arbiter"
        }
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
async fn contracts_endpoint_includes_governance_view() {
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
    assert!(payload["openapi_sha256"].as_str().unwrap().len() == 64);
    assert!(payload["contracts_set_sha256"].as_str().unwrap().len() == 64);
    assert_eq!(
        payload["governance"]["allowed_action_types"],
        json!(["notify", "write_external", "start_job"])
    );
}

#[tokio::test]
async fn events_are_idempotent_for_identical_payload() {
    let app = build_app(test_config()).await.unwrap();
    let event = sample_event("evt-idem");

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

#[tokio::test]
async fn duplicate_event_payload_mismatch_returns_409_with_hashes() {
    let app = build_app(test_config()).await.unwrap();
    let event = sample_event("evt-conflict");
    let first = Request::builder()
        .method("POST")
        .uri("/v1/events")
        .header("content-type", "application/json")
        .body(Body::from(event.to_string()))
        .unwrap();
    let first_res = app.clone().oneshot(first).await.unwrap();
    assert_eq!(first_res.status(), StatusCode::OK);

    let mut changed = sample_event("evt-conflict");
    changed["summary"] = Value::String("changed summary".to_string());
    let second = Request::builder()
        .method("POST")
        .uri("/v1/events")
        .header("content-type", "application/json")
        .body(Body::from(changed.to_string()))
        .unwrap();
    let second_res = app.oneshot(second).await.unwrap();
    assert_eq!(second_res.status(), StatusCode::CONFLICT);
    let body = axum::body::to_bytes(second_res.into_body(), usize::MAX)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["error"]["code"], "conflict.payload_mismatch");
    assert!(payload["error"]["details"]["existing_hash"].is_string());
    assert!(payload["error"]["details"]["incoming_hash"].is_string());
}

#[tokio::test]
async fn duplicate_action_result_payload_mismatch_returns_409_with_hashes() {
    let app = build_app(test_config()).await.unwrap();
    let first = json!({
        "tenant_id": "tenant-a",
        "plan_id": "plan-a",
        "action_id": "act-a",
        "status": "succeeded",
        "occurred_at": "2026-02-14T00:00:00Z",
        "evidence": {"provider_id": "x1"}
    });
    let second = json!({
        "tenant_id": "tenant-a",
        "plan_id": "plan-a",
        "action_id": "act-a",
        "status": "failed",
        "occurred_at": "2026-02-14T00:00:00Z",
        "evidence": {"provider_id": "x2"}
    });

    let first_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/action-results")
                .header("content-type", "application/json")
                .body(Body::from(first.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first_res.status(), StatusCode::NO_CONTENT);

    let second_res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/action-results")
                .header("content-type", "application/json")
                .body(Body::from(second.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second_res.status(), StatusCode::CONFLICT);
    let body = axum::body::to_bytes(second_res.into_body(), usize::MAX)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["error"]["code"], "conflict.payload_mismatch");
}

#[tokio::test]
async fn provider_not_in_allowlist_is_rejected() {
    let app = build_app(test_config()).await.unwrap();
    let mut event = sample_event("evt-provider-deny");
    event["labels"]["provider"] = Value::String("forbidden".to_string());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/events")
                .header("content-type", "application/json")
                .body(Body::from(event.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["error"]["code"], "policy.provider_not_allowed");
}

#[tokio::test]
async fn same_input_produces_same_plan_decision_time() {
    let app = build_app(test_config()).await.unwrap();
    let event = sample_event("evt-deterministic");

    let res1 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/events")
                .header("content-type", "application/json")
                .body(Body::from(event.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let body1 = axum::body::to_bytes(res1.into_body(), usize::MAX)
        .await
        .unwrap();
    let p1: Value = serde_json::from_slice(&body1).unwrap();

    let res2 = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/events")
                .header("content-type", "application/json")
                .body(Body::from(event.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let body2 = axum::body::to_bytes(res2.into_body(), usize::MAX)
        .await
        .unwrap();
    let p2: Value = serde_json::from_slice(&body2).unwrap();

    assert_eq!(p1, p2);
    assert_eq!(
        p1["decision"]["evaluation_time"],
        Value::String("2026-02-14T00:00:00Z".to_string())
    );
}

#[tokio::test]
async fn sqlite_event_idempotency_payload_mismatch_returns_409() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    let db_path = std::env::temp_dir().join(format!("arbiter-sqlite-{nanos}.db"));
    let db_path_str = db_path.to_string_lossy().to_string();

    let app1 = build_app(test_config_sqlite(&db_path_str)).await.unwrap();
    let first = sample_event("evt-sqlite-conflict");
    let first_res = app1
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/events")
                .header("content-type", "application/json")
                .body(Body::from(first.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first_res.status(), StatusCode::OK);

    let app2 = build_app(test_config_sqlite(&db_path_str)).await.unwrap();
    let mut second = sample_event("evt-sqlite-conflict");
    second["summary"] = Value::String("changed".to_string());
    let second_res = app2
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/events")
                .header("content-type", "application/json")
                .body(Body::from(second.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second_res.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn sqlite_action_result_payload_mismatch_returns_409() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    let db_path = std::env::temp_dir().join(format!("arbiter-sqlite-action-result-{nanos}.db"));
    let db_path_str = db_path.to_string_lossy().to_string();

    let app1 = build_app(test_config_sqlite(&db_path_str)).await.unwrap();
    let first = json!({
        "tenant_id": "tenant-a",
        "plan_id": "plan-z",
        "action_id": "act-z",
        "status": "succeeded",
        "occurred_at": "2026-02-14T00:00:00Z",
        "evidence": {"external_id": "1"}
    });
    let first_res = app1
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/action-results")
                .header("content-type", "application/json")
                .body(Body::from(first.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first_res.status(), StatusCode::NO_CONTENT);

    let app2 = build_app(test_config_sqlite(&db_path_str)).await.unwrap();
    let second = json!({
        "tenant_id": "tenant-a",
        "plan_id": "plan-z",
        "action_id": "act-z",
        "status": "failed",
        "occurred_at": "2026-02-14T00:00:00Z",
        "evidence": {"external_id": "2"}
    });
    let second_res = app2
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/action-results")
                .header("content-type", "application/json")
                .body(Body::from(second.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second_res.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn audit_chain_verification_detects_tampering() {
    let cfg = test_config();
    let audit_path = cfg.audit.jsonl_path.clone();
    let app = build_app(cfg).await.unwrap();

    for event_id in ["evt-audit-1", "evt-audit-2"] {
        let event = sample_event(event_id);
        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/events")
                    .header("content-type", "application/json")
                    .body(Body::from(event.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
    }

    assert!(verify_audit_chain(&audit_path).is_ok());

    let mut lines: Vec<String> = std::fs::read_to_string(&audit_path)
        .unwrap()
        .lines()
        .map(|line| line.to_string())
        .collect();
    let mut tampered: Value = serde_json::from_str(&lines[1]).unwrap();
    tampered["kind"] = Value::String("tampered".to_string());
    lines[1] = serde_json::to_string(&tampered).unwrap();
    std::fs::write(&audit_path, format!("{}\n", lines.join("\n"))).unwrap();

    assert!(verify_audit_chain(&audit_path).is_err());
}

#[tokio::test]
async fn audit_chain_verification_with_mirror_succeeds_when_equal() {
    let mut cfg = test_config();
    let mirror_path = cfg.audit.jsonl_path.clone() + ".mirror";
    cfg.audit.immutable_mirror_path = Some(mirror_path.clone());
    let audit_path = cfg.audit.jsonl_path.clone();

    let app = build_app(cfg).await.unwrap();
    let event = sample_event("evt-mirror");
    let _ = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/events")
                .header("content-type", "application/json")
                .body(Body::from(event.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(verify_audit_chain_with_mirror(&audit_path, Some(&mirror_path)).is_ok());
}
