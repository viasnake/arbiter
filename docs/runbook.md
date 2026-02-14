# Operational Runbook

## Health checks

- `GET /v1/healthz` should return `200 ok`
- CI pipeline (`fmt-check`, `lint`, `test`, `build`) should pass on mainline

## Incident triage order

1. Check latest deploy and config changes
2. Check audit stream continuity (`record_hash`, `prev_hash` chain)
3. Check AuthZ endpoint reachability and contract validity
4. Check sqlite store availability and disk saturation

## Common failure modes

- AuthZ provider timeout/error:
  - Expected result depends on `authz.fail_mode`
  - Verify reason code in audit (`authz_*`)
- Clock skew in incoming events:
  - Gate decisions use server time; investigate source timestamp only for diagnostics
- Duplicate events:
  - Verify idempotency hit via audit and identical `plan_id`
- Invalid action result payloads (`POST /v1/action-results`):
  - Verify required fields (`v`, `plan_id`, `action_id`, `tenant_id`, `status`, `ts`)
  - Verify `status` is one of `succeeded`, `failed`, `skipped`
  - Verify `ts` is RFC3339
- Action result conflict (`POST /v1/action-results` returns `409`):
  - Verify duplicate retries send identical payload for same idempotency key
  - Verify the same `(tenant_id, plan_id, action_id)` key does not change payload
- State lookup during incident triage:
  - `GET /v1/jobs/{tenant_id}/{job_id}`
  - `GET /v1/approvals/{tenant_id}/{approval_id}`
  - `GET /v1/action-results/{tenant_id}/{plan_id}/{action_id}`
  - `GET /v1/contracts` for source hashes (`openapi_sha256`, `contracts_set_sha256`)

## Audit mirror operations

- When `audit.immutable_mirror_path` is configured, write failures are fail-closed.
- Verify both chains together:

  ```bash
  arbiter audit-verify --path ./arbiter-audit.jsonl --mirror-path ./arbiter-audit-mirror.jsonl
  ```

- Mirror mismatch indicates integrity divergence; investigate storage and filesystem reliability before replay.

## Recovery guidance

- Prefer no-op outcomes over implicit retries when state is uncertain
- Keep action execution outside Arbiter; replay through event boundary
- For sqlite corruption risk, snapshot DB before repair operations
