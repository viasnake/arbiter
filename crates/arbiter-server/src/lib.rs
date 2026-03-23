mod audit;
mod contracts;
mod errors;
mod handlers;
mod store;

use arbiter_config::Config;
use axum::routing::{get, post};
use axum::Router;
use std::net::SocketAddr;

use crate::handlers::{
    cancel_approval, create_operation_request, deny_approval, get_contracts, get_run,
    get_run_audit, grant_approval, healthz, submit_step_intent, submit_step_result,
};
use crate::store::AppState;

pub use audit::{verify_audit_chain, verify_audit_chain_with_mirror};

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
        .route("/v1/contracts", get(get_contracts))
        .route("/v1/operation-requests", post(create_operation_request))
        .route("/v1/runs/{run_id}", get(get_run))
        .route("/v1/runs/{run_id}/step-intents", post(submit_step_intent))
        .route("/v1/runs/{run_id}/step-results", post(submit_step_result))
        .route("/v1/audit/runs/{run_id}", get(get_run_audit))
        .route("/v1/approvals/{approval_id}/grant", post(grant_approval))
        .route("/v1/approvals/{approval_id}/deny", post(deny_approval))
        .route("/v1/approvals/{approval_id}/cancel", post(cancel_approval))
        .with_state(state))
}

pub async fn doctor(cfg: Config) -> Result<Vec<String>, String> {
    let state = AppState::new(cfg)?;
    let store = state.lock_store().await;
    store
        .doctor()
        .map_err(|err| format!("doctor failed: {err:?}"))
}
