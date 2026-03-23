use arbiter_contracts::{
    Approval, ApprovalActionRequest, ApprovalStatus, AuditRunEventsResponse, ContractsMetadata,
    Decision, DecisionEffect, ExecutionPermit, OperationRequest, OperationRequestAccepted, Run,
    RunEnvelope, RunStatus, Step, StepIntent, StepResultResponse, StepResultSubmission, StepStatus,
};
use axum::extract::{Path as AxPath, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{Duration, Utc};
use serde::de::DeserializeOwned;
use serde_json::json;
use uuid::Uuid;

use arbiter_kernel::jcs_sha256_hex;
use arbiter_kernel::policy::{evaluate, resolve_approvers, PolicyInput};
use arbiter_kernel::state_machine::{
    can_transition_approval, can_transition_run, can_transition_step,
};

use crate::audit::{list_run_events, AuditRecord};
use crate::errors::{into_error, ApiErrorResponse, ApiFailure};
use crate::store::AppState;

pub(crate) async fn healthz() -> (StatusCode, &'static str) {
    (StatusCode::OK, "ok")
}

pub(crate) async fn get_contracts(State(state): State<AppState>) -> Json<ContractsMetadata> {
    Json(state.contracts_metadata())
}

pub(crate) async fn create_operation_request(
    State(state): State<AppState>,
    Json(input): Json<OperationRequest>,
) -> Result<(StatusCode, Json<OperationRequestAccepted>), ApiErrorResponse> {
    let payload_hash = payload_hash(&input)?;
    let idem_key = format!("operation_request:{}", input.request_id);

    let mut store = state.lock_store().await;
    if let Some(idem) = store.get_idempotency(&idem_key).map_err(into_error)? {
        if idem.payload_hash == payload_hash {
            let response: OperationRequestAccepted = decode_snapshot(&idem.response_json)?;
            return Ok((StatusCode::CREATED, Json(response)));
        }
        return Err(into_error(ApiFailure::conflict(
            "conflict",
            "duplicate with different payload",
        )));
    }

    if store
        .find_run_by_request_id(&input.request_id)
        .map_err(into_error)?
        .is_some()
    {
        return Err(into_error(ApiFailure::conflict(
            "conflict",
            "duplicate with same payload",
        )));
    }

    let run_id = format!("run_{}", Uuid::new_v4().simple());
    let now = Utc::now().to_rfc3339();
    let run = Run {
        run_id: run_id.clone(),
        request_id: input.request_id,
        requester: input.requester,
        source: input.source,
        objective: input.objective,
        environment: input
            .environment_hint
            .unwrap_or_else(|| "unknown".to_string()),
        status: RunStatus::Accepted,
        created_at: now.clone(),
        updated_at: now,
        risk_summary: json!({"initial": "unclassified"}),
    };
    store
        .put_run(RunEnvelope {
            run,
            steps: vec![],
            approvals: vec![],
            permits: vec![],
        })
        .map_err(into_error)?;

    let response = OperationRequestAccepted {
        run_id: run_id.clone(),
        status: RunStatus::Accepted,
        decision_summary: "request accepted and queued for planning".to_string(),
        links: std::collections::BTreeMap::from([
            ("run".to_string(), format!("/v1/runs/{run_id}")),
            (
                "step_intents".to_string(),
                format!("/v1/runs/{run_id}/step-intents"),
            ),
        ]),
    };

    store
        .put_idempotency(
            &idem_key,
            &payload_hash,
            &serde_json::to_string(&response)
                .map_err(|err| into_error(ApiFailure::internal(&err.to_string())))?,
        )
        .map_err(into_error)?;

    store
        .append_audit(AuditRecord::new(
            "operation_request_created",
            &run_id,
            "requester",
            json!({"run_id": run_id}),
        ))
        .map_err(into_error)?;

    Ok((StatusCode::CREATED, Json(response)))
}

pub(crate) async fn get_run(
    State(state): State<AppState>,
    AxPath(run_id): AxPath<String>,
) -> Result<Json<RunEnvelope>, ApiErrorResponse> {
    let store = state.lock_store().await;
    let run = store
        .get_run(&run_id)
        .map_err(into_error)?
        .ok_or_else(|| ApiFailure::not_found("not_found", "run not found"))
        .map_err(into_error)?;
    Ok(Json(run))
}

pub(crate) async fn submit_step_intent(
    State(state): State<AppState>,
    AxPath(run_id): AxPath<String>,
    Json(intent): Json<StepIntent>,
) -> Result<Json<Step>, ApiErrorResponse> {
    let id_component = intent
        .client_step_id
        .clone()
        .or(intent.step_id.clone())
        .ok_or_else(|| {
            into_error(ApiFailure::bad_request(
                "invalid_request",
                "step_id or client_step_id is required",
            ))
        })?;
    let idem_key = format!("step_intent:{run_id}:{id_component}");
    let payload_hash = payload_hash(&intent)?;

    let policy_cfg = state.policy_config().clone();
    let approver_cfg = state.approver_config().clone();
    let permit_ttl = state.permit_ttl_seconds();

    let mut store = state.lock_store().await;
    if let Some(idem) = store.get_idempotency(&idem_key).map_err(into_error)? {
        if idem.payload_hash == payload_hash {
            let response: Step = decode_snapshot(&idem.response_json)?;
            return Ok(Json(response));
        }
        return Err(into_error(ApiFailure::conflict(
            "conflict",
            "duplicate with different payload",
        )));
    }

    let mut run = store
        .get_run(&run_id)
        .map_err(into_error)?
        .ok_or_else(|| ApiFailure::not_found("not_found", "run not found"))
        .map_err(into_error)?;

    transition_run(&mut run.run.status, RunStatus::Planning)?;

    let step_id = intent
        .step_id
        .clone()
        .unwrap_or_else(|| format!("step_{}", Uuid::new_v4().simple()));
    let mut step = Step {
        step_id: step_id.clone(),
        run_id: run_id.clone(),
        status: StepStatus::Declared,
        intent: intent.clone(),
        decision: Decision {
            decision_id: format!("dec_{}", Uuid::new_v4().simple()),
            effect: DecisionEffect::Allow,
            rationale: "pending evaluation".to_string(),
            applied_policies: vec![],
            permit_constraints: json!({}),
            required_approvers: vec![],
        },
        permit: None,
        approval_id: None,
        created_at: Utc::now().to_rfc3339(),
        updated_at: None,
    };
    transition_step(&mut step.status, StepStatus::Evaluating)?;

    let approvers = resolve_approvers(&run.run.environment, &approver_cfg);
    let policy = evaluate(
        &PolicyInput {
            provider: intent.provider.clone(),
            capability: intent.capability.clone(),
            intent_type: intent.intent_type.clone(),
            risk_level: intent.risk_level.clone(),
            metadata: intent.metadata.clone(),
        },
        &run.run.environment,
        &policy_cfg,
        approvers,
    );
    step.decision = Decision {
        decision_id: format!("dec_{}", Uuid::new_v4().simple()),
        effect: policy.effect.clone(),
        rationale: policy.rationale,
        applied_policies: policy.applied_policies,
        permit_constraints: policy.permit_constraints.clone(),
        required_approvers: policy.required_approvers.clone(),
    };

    match policy.effect {
        DecisionEffect::Deny => {
            transition_step(&mut step.status, StepStatus::Rejected)?;
            transition_run(&mut run.run.status, RunStatus::Blocked)?;
        }
        DecisionEffect::RequireApproval => {
            transition_step(&mut step.status, StepStatus::ApprovalRequired)?;
            transition_run(&mut run.run.status, RunStatus::WaitingForApproval)?;
            let approval_id = format!("apr_{}", Uuid::new_v4().simple());
            let approval = Approval {
                approval_id: approval_id.clone(),
                run_id: run_id.clone(),
                step_id: step_id.clone(),
                status: ApprovalStatus::Requested,
                required_approvers: step.decision.required_approvers.clone(),
                reason: step.decision.rationale.clone(),
                created_at: Utc::now().to_rfc3339(),
                decided_at: None,
                decided_by: None,
            };
            step.approval_id = Some(approval_id.clone());
            run.approvals.push(approval);
            store
                .map_approval_to_run(&approval_id, &run_id)
                .map_err(into_error)?;
        }
        DecisionEffect::Allow => {
            transition_step(&mut step.status, StepStatus::Permitted)?;
            transition_run(&mut run.run.status, RunStatus::Ready)?;
            let permit = issue_permit(
                &run_id,
                &step_id,
                permit_ttl,
                step.decision.permit_constraints.clone(),
            );
            step.permit = Some(permit.clone());
            run.permits.push(permit);
        }
    }

    step.updated_at = Some(Utc::now().to_rfc3339());
    run.steps.push(step.clone());
    run.run.updated_at = Utc::now().to_rfc3339();
    store.put_run(run).map_err(into_error)?;

    store
        .put_idempotency(
            &idem_key,
            &payload_hash,
            &serde_json::to_string(&step)
                .map_err(|err| into_error(ApiFailure::internal(&err.to_string())))?,
        )
        .map_err(into_error)?;

    let mut audit = AuditRecord::new(
        "step_intent_declared",
        &run_id,
        "planner",
        json!({"step_id": step.step_id, "effect": step.decision.effect}),
    );
    audit.step_id = Some(step.step_id.clone());
    audit.rationale = Some(step.decision.rationale.clone());
    audit.policy_refs = step.decision.applied_policies.clone();
    store.append_audit(audit).map_err(into_error)?;

    Ok(Json(step))
}

pub(crate) async fn grant_approval(
    State(state): State<AppState>,
    AxPath(approval_id): AxPath<String>,
    Json(input): Json<ApprovalActionRequest>,
) -> Result<Json<Approval>, ApiErrorResponse> {
    apply_approval_action(state, approval_id, input, ApprovalStatus::Granted).await
}

pub(crate) async fn deny_approval(
    State(state): State<AppState>,
    AxPath(approval_id): AxPath<String>,
    Json(input): Json<ApprovalActionRequest>,
) -> Result<Json<Approval>, ApiErrorResponse> {
    apply_approval_action(state, approval_id, input, ApprovalStatus::Denied).await
}

pub(crate) async fn cancel_approval(
    State(state): State<AppState>,
    AxPath(approval_id): AxPath<String>,
    Json(input): Json<ApprovalActionRequest>,
) -> Result<Json<Approval>, ApiErrorResponse> {
    apply_approval_action(state, approval_id, input, ApprovalStatus::Cancelled).await
}

pub(crate) async fn submit_step_result(
    State(state): State<AppState>,
    AxPath(run_id): AxPath<String>,
    Json(input): Json<StepResultSubmission>,
) -> Result<Json<StepResultResponse>, ApiErrorResponse> {
    let idem_key = format!("step_result:{run_id}:{}", input.step_id);
    let payload_hash = payload_hash(&input)?;

    let mut store = state.lock_store().await;
    if let Some(idem) = store.get_idempotency(&idem_key).map_err(into_error)? {
        if idem.payload_hash == payload_hash {
            let response: StepResultResponse = decode_snapshot(&idem.response_json)?;
            return Ok(Json(response));
        }
        return Err(into_error(ApiFailure::conflict(
            "conflict",
            "duplicate with different payload",
        )));
    }

    let mut run = store
        .get_run(&run_id)
        .map_err(into_error)?
        .ok_or_else(|| ApiFailure::not_found("not_found", "run not found"))
        .map_err(into_error)?;

    let step = run
        .steps
        .iter_mut()
        .find(|v| v.step_id == input.step_id)
        .ok_or_else(|| ApiFailure::not_found("not_found", "step not found"))
        .map_err(into_error)?;

    if step.status == StepStatus::ApprovalRequired {
        return Err(into_error(ApiFailure::approval_required(
            "approval_required",
            "step requires approval before result submission",
        )));
    }

    if step.status == StepStatus::Permitted {
        transition_step(&mut step.status, StepStatus::Executing)?;
    }

    if input.error.is_some() {
        transition_step(&mut step.status, StepStatus::Failed)?;
        transition_run(&mut run.run.status, RunStatus::Running)?;
        transition_run(&mut run.run.status, RunStatus::Failed)?;
    } else {
        transition_step(&mut step.status, StepStatus::Completed)?;
        if run.run.status == RunStatus::Ready {
            transition_run(&mut run.run.status, RunStatus::Running)?;
        }
        transition_run(&mut run.run.status, RunStatus::Succeeded)?;
    }

    step.updated_at = Some(Utc::now().to_rfc3339());
    run.run.updated_at = Utc::now().to_rfc3339();
    let response = StepResultResponse {
        step_status: step.status.clone(),
        run_status: run.run.status.clone(),
    };

    store.put_run(run).map_err(into_error)?;
    store
        .put_idempotency(
            &idem_key,
            &payload_hash,
            &serde_json::to_string(&response)
                .map_err(|err| into_error(ApiFailure::internal(&err.to_string())))?,
        )
        .map_err(into_error)?;

    let mut audit = AuditRecord::new(
        "step_result_recorded",
        &run_id,
        "executor",
        json!({"step_id": input.step_id, "execution_result": input.execution_result}),
    );
    audit.step_id = Some(input.step_id);
    store.append_audit(audit).map_err(into_error)?;

    Ok(Json(response))
}

pub(crate) async fn get_run_audit(
    State(state): State<AppState>,
    AxPath(run_id): AxPath<String>,
) -> Result<Json<AuditRunEventsResponse>, ApiErrorResponse> {
    let store = state.lock_store().await;
    let payload = list_run_events(store.audit_path(), &run_id).map_err(into_error)?;
    Ok(Json(payload))
}

async fn apply_approval_action(
    state: AppState,
    approval_id: String,
    input: ApprovalActionRequest,
    target: ApprovalStatus,
) -> Result<Json<Approval>, ApiErrorResponse> {
    let idem_key = format!(
        "approval_action:{approval_id}:{}",
        match target {
            ApprovalStatus::Granted => "grant",
            ApprovalStatus::Denied => "deny",
            ApprovalStatus::Cancelled => "cancel",
            ApprovalStatus::Requested => "requested",
        }
    );
    let payload_hash = payload_hash(&input)?;
    let permit_ttl = state.permit_ttl_seconds();

    let mut store = state.lock_store().await;
    if let Some(idem) = store.get_idempotency(&idem_key).map_err(into_error)? {
        if idem.payload_hash == payload_hash {
            let response: Approval = decode_snapshot(&idem.response_json)?;
            return Ok(Json(response));
        }
        return Err(into_error(ApiFailure::conflict(
            "conflict",
            "duplicate with different payload",
        )));
    }

    let run_id = store
        .run_id_for_approval(&approval_id)
        .map_err(into_error)?
        .ok_or_else(|| ApiFailure::not_found("not_found", "approval not found"))
        .map_err(into_error)?;
    let mut run = store
        .get_run(&run_id)
        .map_err(into_error)?
        .ok_or_else(|| ApiFailure::not_found("not_found", "run not found"))
        .map_err(into_error)?;

    let approval = run
        .approvals
        .iter_mut()
        .find(|v| v.approval_id == approval_id)
        .ok_or_else(|| ApiFailure::not_found("not_found", "approval not found"))
        .map_err(into_error)?;
    if approval.status == target {
        let snapshot = approval.clone();
        store
            .put_idempotency(
                &idem_key,
                &payload_hash,
                &serde_json::to_string(&snapshot)
                    .map_err(|err| into_error(ApiFailure::internal(&err.to_string())))?,
            )
            .map_err(into_error)?;
        return Ok(Json(snapshot));
    }
    if !can_transition_approval(&approval.status, &target) {
        let reason = match approval.status {
            ApprovalStatus::Granted => "already approved",
            ApprovalStatus::Denied => "already denied",
            ApprovalStatus::Cancelled => "already cancelled",
            ApprovalStatus::Requested => "invalid state transition",
        };
        return Err(into_error(ApiFailure::conflict("conflict", reason)));
    }

    approval.status = target.clone();
    approval.decided_at = Some(Utc::now().to_rfc3339());
    approval.decided_by = Some(input.actor.clone());

    let step = run
        .steps
        .iter_mut()
        .find(|v| v.approval_id.as_deref() == Some(approval_id.as_str()))
        .ok_or_else(|| ApiFailure::not_found("not_found", "step not found"))
        .map_err(into_error)?;
    match target {
        ApprovalStatus::Granted => {
            transition_step(&mut step.status, StepStatus::Permitted)?;
            transition_run(&mut run.run.status, RunStatus::Ready)?;
            let permit = issue_permit(
                &run_id,
                &step.step_id,
                permit_ttl,
                json!({"approved": true}),
            );
            step.permit = Some(permit.clone());
            run.permits.push(permit);
        }
        ApprovalStatus::Denied => {
            transition_step(&mut step.status, StepStatus::Rejected)?;
            transition_run(&mut run.run.status, RunStatus::Blocked)?;
        }
        ApprovalStatus::Cancelled => {
            transition_step(&mut step.status, StepStatus::Cancelled)?;
            transition_run(&mut run.run.status, RunStatus::Cancelled)?;
        }
        ApprovalStatus::Requested => {}
    }

    run.run.updated_at = Utc::now().to_rfc3339();
    let snapshot = approval.clone();
    store.put_run(run).map_err(into_error)?;
    store
        .put_idempotency(
            &idem_key,
            &payload_hash,
            &serde_json::to_string(&snapshot)
                .map_err(|err| into_error(ApiFailure::internal(&err.to_string())))?,
        )
        .map_err(into_error)?;

    let mut audit = AuditRecord::new(
        "approval_decided",
        &run_id,
        &input.actor,
        json!({"approval_id": approval_id, "status": snapshot.status}),
    );
    audit.approval_id = Some(snapshot.approval_id.clone());
    audit.step_id = Some(snapshot.step_id.clone());
    audit.rationale = input.reason;
    store.append_audit(audit).map_err(into_error)?;

    Ok(Json(snapshot))
}

fn issue_permit(
    run_id: &str,
    step_id: &str,
    ttl_seconds: u64,
    constraints: serde_json::Value,
) -> ExecutionPermit {
    let issued = Utc::now();
    ExecutionPermit {
        permit_id: format!("permit_{}", Uuid::new_v4().simple()),
        run_id: run_id.to_string(),
        step_id: step_id.to_string(),
        issuer: "arbiter".to_string(),
        constraints,
        issued_at: issued.to_rfc3339(),
        expires_at: (issued + Duration::seconds(ttl_seconds as i64)).to_rfc3339(),
    }
}

fn payload_hash<T: serde::Serialize>(payload: &T) -> Result<String, ApiErrorResponse> {
    let value = serde_json::to_value(payload).map_err(|err| {
        into_error(ApiFailure::internal(&format!(
            "payload encode failed: {err}"
        )))
    })?;
    jcs_sha256_hex(&value)
        .map_err(|err| into_error(ApiFailure::internal(&format!("payload hash failed: {err}"))))
}

fn decode_snapshot<T: DeserializeOwned>(input: &str) -> Result<T, ApiErrorResponse> {
    serde_json::from_str(input).map_err(|err| {
        into_error(ApiFailure::internal(&format!(
            "idempotency snapshot decode failed: {err}"
        )))
    })
}

fn transition_run(current: &mut RunStatus, next: RunStatus) -> Result<(), ApiErrorResponse> {
    if current == &next {
        return Ok(());
    }
    if !can_transition_run(current, &next) {
        return Err(into_error(ApiFailure::invalid_transition(
            "invalid_transition",
            "invalid run state transition",
        )));
    }
    *current = next;
    Ok(())
}

fn transition_step(current: &mut StepStatus, next: StepStatus) -> Result<(), ApiErrorResponse> {
    if current == &next {
        return Ok(());
    }
    if !can_transition_step(current, &next) {
        return Err(into_error(ApiFailure::invalid_transition(
            "invalid_transition",
            "invalid step state transition",
        )));
    }
    *current = next;
    Ok(())
}
