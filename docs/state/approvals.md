# Approval State Semantics

## State model

- Key: `(tenant_id, approval_id)`
- Fields: `status`, `reason_code`, `updated_at`
- Status enum: `requested | approved | rejected | expired`

## Event idempotency

- Ingest endpoint: `POST /v1/approval-events`
- Idempotency key: `(tenant_id, event_id)`
- Duplicate with same payload: accepted and treated as idempotent replay
- Duplicate with different payload: `409` with `error.code=conflict.payload_mismatch`

## Transition rules

- `requested` may transition to `approved`, `rejected`, or `expired`.
- Terminal states (`approved`, `rejected`, `expired`) are immutable.
- Invalid transitions return `409` with `error.code=conflict.invalid_transition`.

## Expiration policy

- `expired` is set only by explicit `ApprovalEvent(status=expired)`.
- `planner.approval_timeout_ms` is advisory metadata for planning and does not trigger background expiration.

## Read path

- `GET /v1/approvals/{tenant_id}/{approval_id}` returns latest persisted state.
