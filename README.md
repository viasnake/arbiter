# Arbiter

Arbiter is a provider-agnostic governance plane for AI agent operations.

Arbiter governs operations through:

`request -> run -> step -> approval -> permit -> result -> audit`

It is not an LLM runtime, scheduler, or knowledge pipeline.

## Responsibilities

- Request intake and run orchestration
- Policy evaluation (allow/deny/require_approval)
- Approval mediation and permit issuance
- Idempotency and conflict handling
- Deterministic state transitions
- Tamper-evident audit chain

## Non-responsibilities

- LLM reasoning and model selection
- Tool execution implementation
- Knowledge ETL/indexing/retrieval engine
- Job scheduler and background workflow runtime

## Public API (`/v1` only)

- `GET /v1/healthz`
- `GET /v1/contracts`
- `POST /v1/operation-requests`
- `GET /v1/runs/{run_id}`
- `POST /v1/runs/{run_id}/step-intents`
- `POST /v1/runs/{run_id}/step-results`
- `POST /v1/approvals/{approval_id}/grant`
- `POST /v1/approvals/{approval_id}/deny`
- `POST /v1/approvals/{approval_id}/cancel`
- `GET /v1/audit/runs/{run_id}`

OpenAPI source of truth: `openapi/v1.yaml`

## Run and Step state machines

Run status:

- `accepted`
- `planning`
- `waiting_for_approval`
- `ready`
- `running`
- `blocked`
- `succeeded`
- `failed`
- `cancelled`

Step status:

- `declared`
- `evaluating`
- `approval_required`
- `permitted`
- `executing`
- `completed`
- `rejected`
- `failed`
- `cancelled`

## Idempotency

- Operation requests: keyed by `request_id`
- Step intents: keyed by `run_id + (client_step_id|step_id)`
- Approval actions: keyed by `approval_id + action`
- Step results: keyed by `run_id + step_id`

If same key + same payload is retried, Arbiter returns the original response.
If same key + different payload is submitted, Arbiter returns `409 conflict`.

## Audit chain

- Audit entries are append-only JSONL
- Every entry includes `prev_hash` and `hash`
- Hashes are computed from canonical JSON
- On startup, Arbiter restores the last hash from existing audit log

Verify:

```bash
arbiter audit-verify --path ./arbiter-audit.jsonl --mirror-path ./arbiter-audit-mirror.jsonl
```

## Configuration example

See `config/example-config.yaml`.

Key enforced settings:

- `governance.allowed_providers`
- `governance.capability_allowlist/denylist`
- `governance.permit_ttl_seconds`
- `policy.require_approval_for_*`
- `approver.default_approvers` / `approver.production_approvers`
- `store.kind` (`memory` or `sqlite`)
- `audit.jsonl_path`

## CLI

- `arbiter serve --config ./config/example-config.yaml`
- `arbiter config-validate --config ./config/example-config.yaml`
- `arbiter audit-verify --path ./arbiter-audit.jsonl --mirror-path ./arbiter-audit-mirror.jsonl`
- `arbiter store-doctor --config ./config/example-config.yaml`

## Verify locally

```bash
mise run version-check
mise run fmt-check
mise run lint
mise run contracts-verify
mise run test
mise run build
```

## Documentation

- Specification: `docs/spec.md`
