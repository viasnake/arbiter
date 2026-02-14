# Operational Runbook (v1.0.0 Target)

## Health checks

- `GET /v1/healthz` must return `200 ok`
- CI pipeline (`fmt-check`, `lint`, `test`, `build`) must pass on mainline

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

## Recovery guidance

- Prefer no-op outcomes over implicit retries when state is uncertain
- Keep action execution outside Arbiter; replay through event boundary
- For sqlite corruption risk, snapshot DB before repair operations
