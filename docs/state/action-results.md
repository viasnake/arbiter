# Action-Results Semantics

## Purpose

`POST /v1/action-results` reports execution outcomes back to Arbiter.

## State key

- Primary key: `(tenant_id, plan_id, action_id)`

## Ingest behavior

- First write wins.
- Duplicate with identical payload is accepted (`204`).
- Same key with different payload returns `409` with `error.code=conflict.payload_mismatch`.

## Terminal statuses

- `succeeded`, `failed`, `skipped` are terminal report values in v1.1.0.

## Retrieval

- `GET /v1/action-results/{tenant_id}/{plan_id}/{action_id}`
- `404` when not found.

## Executor guidance

- Retries for the same action result should resend identical payload.
- Include stable `ts` and status values for duplicate retry safety.
