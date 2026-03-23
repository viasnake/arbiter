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
    create_run, get_contracts, get_run, grant_approval, healthz, submit_step_intent,
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
    let state = AppState::new(cfg);
    Ok(Router::new()
        .route("/v1/healthz", get(healthz))
        .route("/v1/contracts", get(get_contracts))
        .route("/v2/operation-requests", post(create_run))
        .route("/v2/runs/{run_id}", get(get_run))
        .route("/v2/runs/{run_id}/step-intents", post(submit_step_intent))
        .route("/v2/approvals/{approval_id}/grant", post(grant_approval))
        .with_state(state))
}
