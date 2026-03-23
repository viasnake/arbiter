# Arbiter Specification

This document defines the implemented runtime behavior.

## Runtime Endpoints

### `GET /v1/healthz`

- Returns `200` with plain text body `ok`.

### `GET /v1/contracts`

- Returns `ContractsMetadata` generated at build time.

### `POST /v1/operation-requests`

- Input: `OperationRequest`
- Creates one `Run`
- Returns `201` with `OperationRequestAccepted`
- Idempotent by `request_id`

### `GET /v1/runs/{run_id}`

- Returns `RunEnvelope` (`run`, `steps`, `approvals`, `permits`)

### `POST /v1/runs/{run_id}/step-intents`

- Input: `StepIntent`
- Evaluates policy and transitions step/run state
- Returns `Step`
- Idempotent by `run_id + (client_step_id|step_id)`

### `POST /v1/approvals/{approval_id}/grant`
### `POST /v1/approvals/{approval_id}/deny`
### `POST /v1/approvals/{approval_id}/cancel`

- Input: `ApprovalActionRequest`
- Applies approval state transition and updates related step/run
- Returns updated `Approval`
- Idempotent per `(approval_id, action)`

### `POST /v1/runs/{run_id}/step-results`

- Input: `StepResultSubmission`
- Updates step/run completion state
- Returns `StepResultResponse`
- Idempotent by `run_id + step_id`

### `GET /v1/audit/runs/{run_id}`

- Returns all recorded `AuditEvent` for the run

## State Machines

### Run

- `accepted -> planning`
- `planning -> waiting_for_approval`
- `planning -> ready`
- `waiting_for_approval -> ready`
- `ready -> running`
- `running -> succeeded`
- `running -> failed`
- `* -> cancelled`

Invalid transition returns `422 invalid_transition`.

### Step

- `declared -> evaluating`
- `evaluating -> approval_required|permitted`
- `approval_required -> permitted|rejected`
- `permitted -> executing|failed`
- `executing -> completed|failed`
- `* -> cancelled`

## Policy and Approver Resolution

Policy input includes:

- provider
- capability
- risk_level
- environment
- metadata

Policy output:

- `allow`
- `deny`
- `require_approval`

Approvers are resolved by configuration:

- `approver.default_approvers`
- `approver.production_approvers`

No hardcoded approver identity is used.

## Idempotency and Conflict

Arbiter stores:

- idempotency key
- canonical payload hash
- first response snapshot
- timestamp

Conflict behavior (`409 conflict`):

- duplicate with same payload -> returns original response
- duplicate with different payload -> conflict error

## Audit Integrity

Audit fields include:

- `event_id`
- `event_type`
- `run_id`
- `step_id`
- `approval_id`
- `actor`
- `timestamp`
- `payload_hash`
- `prev_hash`
- `hash`
- `rationale`
- `policy_refs`

Hash chain is restart-safe:

- startup restores last hash from persisted log
- append links to restored `prev_hash`
- `audit-verify` validates entire chain

## Store Backends

- `memory`
- `sqlite`

`sqlite` stores runs, approval mapping, and idempotency records.

## Error Envelope

```json
{
  "error": {
    "code": "string",
    "message": "string",
    "details": null
  }
}
```

Used status codes:

- `400 invalid_request`
- `401 unauthorized`
- `403 forbidden`
- `404 not_found`
- `409 conflict`
- `422 invalid_transition`
- `423 approval_required`
- `500 internal_error`
