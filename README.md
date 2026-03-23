# Arbiter

Arbiter is a governance-oriented control plane for AI operations.

The current runtime model is run-based (`v2` paths), with compatibility endpoints under `v1` for health and contract metadata.

## Naming

- Arbiter "v2" is the current runtime surface.
- `/v1/*` paths are still used for health and contract metadata.

## Runtime API surface

- `GET /v1/healthz`
- `GET /v1/contracts`
- `POST /v2/operation-requests`
- `GET /v2/runs/{run_id}`
- `POST /v2/runs/{run_id}/step-intents`
- `POST /v2/approvals/{approval_id}/grant`

OpenAPI source of truth: `openapi/v1.yaml`

## Runtime behavior

- `POST /v2/operation-requests` creates a run and writes an audit record.
- `POST /v2/runs/{run_id}/step-intents` evaluates a step intent and returns a decision.
- Approval is required only when `step_type=tool_call` and `risk_level` is `write`, `external`, or `high`.
- `POST /v2/approvals/{approval_id}/grant` turns a waiting approval into a permit.
- Audit records are append-only and hash-chained (`prev_hash`, `record_hash`).

## CLI

- `arbiter serve --config ./config/example-config.yaml`
- `arbiter audit-verify --path ./arbiter-audit.jsonl --mirror-path ./arbiter-audit-mirror.jsonl`

## Run locally

```bash
mise install
mise exec -- cargo run -- serve --config ./config/example-config.yaml
```

## Verify

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
