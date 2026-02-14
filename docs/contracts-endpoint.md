# `/v1/contracts` Semantics

## Contract

`GET /v1/contracts` returns transport metadata and source hashes.

Fields:

- `version`: current server package version
- `openapi_sha256`: SHA-256 of `openapi/v1.yaml` bytes
- `contracts_set_sha256`: SHA-256 of canonical contract set concatenation
- `generated_at`: build timestamp (RFC3339, UTC)
- `actions.enabled`: action types Arbiter may emit
- `actions.reserved`: recognized but currently non-emitted action types
- `inputs.job_events`: accepted job event statuses from schema enum
- `inputs.approval_events`: accepted approval event statuses from schema enum
- `schemas`: map of canonical schema path to per-file SHA-256

## Determinism boundary

- Hash fields are stable for the same repository source state.
- `generated_at` reflects build time and is expected to differ across separate builds.

## Source of truth

The endpoint is generated from build-time manifest data and should not duplicate manual schema lists.
