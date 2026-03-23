# Arbiter Current Specification

This document is the single source of truth for Arbiter runtime behavior.

It describes the current implementation only. Historical release notes and legacy protocol narratives are intentionally excluded.

## Naming

- "Arbiter v2" refers to the current run-based runtime model.
- Runtime APIs are split across `/v1` (health and contracts metadata) and `/v2` (run lifecycle).

## Runtime Endpoints

### `GET /v1/healthz`

- Returns `200` with plain text body `ok`.

### `GET /v1/contracts`

- Returns `200` with contract metadata generated at build time.
- Response fields:
  - `api_version`
  - `openapi_sha256`
  - `contracts_set_sha256`
  - `generated_at`
  - `schemas` (map: canonical schema path -> sha256)

### `POST /v2/operation-requests`

- Request body: `OperationRequest`.
- Creates a new run and persists it in memory.
- Returns `201` with `RunEnvelope`.
- Appends audit event `run_created`.

### `GET /v2/runs/{run_id}`

- Returns the current in-memory run envelope.
- `200` on success.
- `404` with error envelope when run does not exist.

### `POST /v2/runs/{run_id}/step-intents`

- Request body: `StepIntent`.
- Evaluates intent and appends a new step to the run.
- Decision rule for approval:
  - approval required only when `step_type == tool_call`
  - and `risk_level` is one of `write`, `external`, `high`
- Returns `200` with `Step`.
- Returns `400` for invalid terminal run state.
- Returns `404` when run does not exist.

### `POST /v2/approvals/{approval_id}/grant`

- Grants a pending approval.
- Updates related step/request status and issues an execution permit.
- Returns `204` on success.
- Returns `404` when approval or run does not exist.

## Error Envelope

Error responses use a stable JSON envelope:

```json
{
  "error": {
    "code": "string",
    "message": "string",
    "details": null
  }
}
```

Current code families used by runtime handlers include:

- `run.not_found`
- `approval.not_found`
- `run.invalid_state`
- `internal.error`

## Data and State

- Runtime store is currently in-memory (`HashMap`) for runs and approval mapping.
- State is process-local and is reset on restart.
- Audit stream is append-only JSONL and includes a hash chain:
  - `prev_hash`
  - `record_hash`

## Audit Integrity

- Every mutating runtime action appends an audit record.
- If `audit.immutable_mirror_path` is configured, each record is also appended to mirror output.
- Verification command:

```bash
arbiter audit-verify --path ./arbiter-audit.jsonl --mirror-path ./arbiter-audit-mirror.jsonl
```

## Configuration Reality

- Config schema accepts `store.kind = memory|sqlite`.
- Current server runtime always uses in-memory store implementation.
- Governance and policy values are loaded and validated from config.
- Current step approval decision uses request payload (`step_type`, `risk_level`) and does not read policy toggles from config.

## Non-Goals of Current Runtime

- No connector runtime.
- No executor runtime.
- No background scheduler.
- No automatic state transitions outside explicit API calls.

## Authoritative References

- Runtime routing: `crates/arbiter-server/src/lib.rs`
- Runtime handlers: `crates/arbiter-server/src/handlers.rs`
- Runtime state store: `crates/arbiter-server/src/store.rs`
- Audit implementation: `crates/arbiter-server/src/audit.rs`
- Shared contracts/types: `crates/arbiter-contracts/src/lib.rs`
- OpenAPI document: `openapi/v1.yaml`
