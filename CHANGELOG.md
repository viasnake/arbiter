# Changelog

## 1.1.0

### Added

- `GET /v1/action-results/{tenant_id}/{plan_id}/{action_id}` for stored action-result retrieval.
- Build-time generated contracts metadata manifest in `arbiter-contracts`.
- State semantics docs for jobs, approvals, and action-results.
- Audit mirror semantics doc and mirror verification CLI support.
- Contracts verification task in local CI (`mise run contracts-verify`).

### Changed

- `/v1/contracts` now reads build-generated metadata from contracts + OpenAPI source files.
- Job and approval event processing now enforces transition conflicts (`409`).
- Job and approval duplicate events with payload mismatch return `409 conflict.payload_mismatch`.
- Action-results idempotency key is now `(tenant_id, plan_id, action_id)`.
- Audit mirror write failures are fail-closed for request processing.

### Non-goals

- No background expiration mutation for approvals.
- No external action execution in Arbiter runtime.
