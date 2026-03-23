use arbiter_contracts::{
    ApprovalRequest, ContractsMetadata, Decision, DecisionEffect, ExecutionPermit,
    OperationRequest, Run, RunEnvelope, RunStatus, Step, StepIntent, StepStatus, StepType,
};
use axum::extract::{Path as AxPath, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{Duration, Utc};
use serde_json::json;
use uuid::Uuid;

use crate::audit::AuditRecord;
use crate::errors::{into_error, ApiErrorResponse, ApiFailure};
use crate::store::AppState;

pub(crate) async fn healthz() -> (StatusCode, &'static str) {
    (StatusCode::OK, "ok")
}

pub(crate) async fn get_contracts(State(state): State<AppState>) -> Json<ContractsMetadata> {
    Json(state.contracts_metadata())
}

pub(crate) async fn create_run(
    State(state): State<AppState>,
    Json(input): Json<OperationRequest>,
) -> Result<(StatusCode, Json<RunEnvelope>), ApiErrorResponse> {
    let now = Utc::now().to_rfc3339();
    let run_id = format!("run_{}", Uuid::new_v4().simple());
    let run = Run {
        run_id: run_id.clone(),
        request_id: input.request_id.clone(),
        agent_id: input.target_agent,
        executor_id: "executor-default".to_string(),
        status: RunStatus::Running,
        created_at: now.clone(),
        started_at: Some(now),
        completed_at: None,
        policy_snapshot: json!({"mode": "policy_pipeline"}),
        budget_snapshot: json!({}),
        lease_owner: "executor-default".to_string(),
    };
    let envelope = RunEnvelope { run, steps: vec![] };

    let mut store = state.lock_store().await;
    store.insert_run(run_id.clone(), envelope.clone());
    store
        .append_audit(AuditRecord::new(
            "run_created",
            &run_id,
            json!({"request_id": input.request_id, "requester": input.requester}),
        ))
        .map_err(into_error)?;

    Ok((StatusCode::CREATED, Json(envelope)))
}

pub(crate) async fn get_run(
    State(state): State<AppState>,
    AxPath(run_id): AxPath<String>,
) -> Result<Json<RunEnvelope>, ApiErrorResponse> {
    let store = state.lock_store().await;
    let run = store
        .get_run(&run_id)
        .ok_or_else(|| ApiFailure::not_found("run.not_found", "run not found"))
        .map_err(into_error)?;
    Ok(Json(run))
}

pub(crate) async fn submit_step_intent(
    State(state): State<AppState>,
    AxPath(run_id): AxPath<String>,
    Json(intent): Json<StepIntent>,
) -> Result<Json<Step>, ApiErrorResponse> {
    let mut store = state.lock_store().await;
    let mut run = store
        .take_run(&run_id)
        .ok_or_else(|| ApiFailure::not_found("run.not_found", "run not found"))
        .map_err(into_error)?;

    if run.run.status == RunStatus::Cancelled
        || run.run.status == RunStatus::Completed
        || run.run.status == RunStatus::TimedOut
    {
        return Err(into_error(ApiFailure::bad_request(
            "run.invalid_state",
            "cannot accept step-intent in terminal run state",
        )));
    }

    let needs_approval = intent.step_type == StepType::ToolCall
        && matches!(intent.risk_level.as_str(), "write" | "external" | "high");

    let (decision, permit, approval_request, next_run_status, step_status) = if needs_approval {
        let approval_id = format!("apr_{}", Uuid::new_v4().simple());
        let expires_at = (Utc::now() + Duration::minutes(30)).to_rfc3339();
        let request = ApprovalRequest {
            approval_id: approval_id.clone(),
            run_id: run_id.clone(),
            step_id: format!("step_{}", Uuid::new_v4().simple()),
            requested_action: intent.proposed_action.clone(),
            approver_set: vec!["human-approver".to_string()],
            status: "requested".to_string(),
            reason: "write/external operation requires approval".to_string(),
            expires_at,
        };

        store.map_approval_to_run(approval_id, run_id.clone());

        (
            Decision {
                decision_id: format!("dec_{}", Uuid::new_v4().simple()),
                effect: DecisionEffect::RequireApproval,
                reason: "approval required for write/external step".to_string(),
                applied_policies: vec!["approval.require_for_write".to_string()],
                constraints: json!({}),
                required_approvers: vec!["human-approver".to_string()],
                executor_scope: Some("executor-default".to_string()),
                expires_at: None,
            },
            None,
            Some(request),
            RunStatus::WaitingForApproval,
            StepStatus::WaitingForApproval,
        )
    } else {
        let issued = Utc::now();
        let permit = ExecutionPermit {
            permit_id: format!("permit_{}", Uuid::new_v4().simple()),
            run_id: run_id.clone(),
            step_id: format!("step_{}", Uuid::new_v4().simple()),
            executor_id: run.run.executor_id.clone(),
            allowed_action: intent.proposed_action.clone(),
            constraints: json!({"mode": "normal"}),
            issued_at: issued.to_rfc3339(),
            expires_at: (issued + Duration::minutes(5)).to_rfc3339(),
            token: Uuid::new_v4().to_string(),
        };

        (
            Decision {
                decision_id: format!("dec_{}", Uuid::new_v4().simple()),
                effect: DecisionEffect::Allow,
                reason: "policy allows step".to_string(),
                applied_policies: vec!["default_allow".to_string()],
                constraints: json!({}),
                required_approvers: vec![],
                executor_scope: Some("executor-default".to_string()),
                expires_at: Some(permit.expires_at.clone()),
            },
            Some(permit),
            None,
            RunStatus::Running,
            StepStatus::Permitted,
        )
    };

    run.run.status = next_run_status;
    let step = Step {
        step_id: format!("step_{}", Uuid::new_v4().simple()),
        run_id: run_id.clone(),
        step_type: intent.step_type.clone(),
        status: step_status,
        intent,
        decision,
        permit,
        approval_request,
        started_at: Utc::now().to_rfc3339(),
        completed_at: None,
    };
    run.steps.push(step.clone());
    store.put_run(run_id.clone(), run);

    store
        .append_audit(AuditRecord::new(
            "step_intent_received",
            &run_id,
            json!({"step_id": step.step_id, "decision": step.decision.effect}),
        ))
        .map_err(into_error)?;

    Ok(Json(step))
}

pub(crate) async fn grant_approval(
    State(state): State<AppState>,
    AxPath(approval_id): AxPath<String>,
) -> Result<StatusCode, ApiErrorResponse> {
    let mut store = state.lock_store().await;
    let run_id = store
        .run_id_for_approval(&approval_id)
        .ok_or_else(|| ApiFailure::not_found("approval.not_found", "approval not found"))
        .map_err(into_error)?;

    let run = store
        .run_mut(&run_id)
        .ok_or_else(|| ApiFailure::not_found("run.not_found", "run not found"))
        .map_err(into_error)?;
    run.run.status = RunStatus::Running;

    for step in run.steps.iter_mut().rev() {
        if let Some(req) = step.approval_request.as_mut() {
            if req.approval_id == approval_id {
                req.status = "approved".to_string();
                step.status = StepStatus::Permitted;
                let issued = Utc::now();
                step.permit = Some(ExecutionPermit {
                    permit_id: format!("permit_{}", Uuid::new_v4().simple()),
                    run_id: run_id.clone(),
                    step_id: step.step_id.clone(),
                    executor_id: run.run.executor_id.clone(),
                    allowed_action: step.intent.proposed_action.clone(),
                    constraints: json!({"approved": true}),
                    issued_at: issued.to_rfc3339(),
                    expires_at: (issued + Duration::minutes(5)).to_rfc3339(),
                    token: Uuid::new_v4().to_string(),
                });
                break;
            }
        }
    }

    store
        .append_audit(AuditRecord::new(
            "approval_granted",
            &run_id,
            json!({"approval_id": approval_id}),
        ))
        .map_err(into_error)?;

    Ok(StatusCode::NO_CONTENT)
}
