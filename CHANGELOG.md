# Changelog

## 1.1.1

### Added

- Event idempotency semantics document: `docs/state/events.md`.
- API error code catalog: `docs/errors.md`.

### Changed

- `POST /v1/events` now rejects duplicate `(tenant_id, event_id)` with payload mismatch using `409 conflict.payload_mismatch` and hash diagnostics.
- `build_app` now rejects unsupported `store.kind` in all paths with `config.invalid_store_kind`.
- README (EN/JA) now documents event duplicate conflict behavior and links the new state/error docs.

### Fixed

- Added memory/sqlite tests for duplicate event payload mismatch conflict behavior.
- Added test coverage for invalid `store.kind` startup failure.

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
