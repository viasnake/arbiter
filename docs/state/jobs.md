# Job State Semantics

## State model

- Key: `(tenant_id, job_id)`
- Fields: `status`, `reason_code`, `updated_at`
- Status enum: `started | heartbeat | completed | failed | cancelled`

## Event idempotency

- Ingest endpoint: `POST /v1/job-events`
- Idempotency key: `(tenant_id, event_id)`
- Duplicate with same payload: accepted and treated as idempotent replay
- Duplicate with different payload: `409` with `error.code=conflict.payload_mismatch`

## Transition rules

- Non-terminal to terminal transitions are allowed.
- Terminal states (`completed`, `failed`, `cancelled`) are immutable.
- Transition violating this rule returns `409` with `error.code=conflict.invalid_transition`.

## Read path

- `GET /v1/jobs/{tenant_id}/{job_id}` returns latest persisted state.
- No background mutation is applied.
- State changes only through explicit input events.
