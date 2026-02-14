# Arbiter

[English](README.md) | [日本語](README.ja.md)

Arbiter is a deterministic decision control plane for AI-driven products.

It decides what should happen next. It does not execute that action.

## Why this exists

Most AI incidents are not caused by weak text generation quality. They are caused by weak control: duplicate execution, unclear authorization, hidden retries, and missing audit evidence.

Arbiter exists to make decision behavior explicit, repeatable, and diagnosable.

When generation, policy, and execution are tightly coupled in one runtime, operators cannot reliably answer basic questions after an incident:

- Why was this action allowed?
- Why did the system choose this path?
- Why did a retry produce different behavior?

Arbiter separates decision logic from execution so those questions can be answered with evidence.

## Typical use cases

Arbiter is useful when side effects are expensive or risky.

- Messaging and assistant systems where duplicate sends must be prevented.
- Human-in-the-loop workflows that require explicit approval before action.
- Multi-tenant systems that need consistent gate and authorization policy enforcement.
- Long-running agent workflows that need job state and cancellation control.
- Environments that require auditable, deterministic replay for incident analysis.

## What Arbiter can do

- Validate normalized input events against contracts.
- Enforce gate decisions (cooldown, queue, rate).
- Evaluate authorization and fail posture.
- Produce deterministic response plans.
- Guarantee idempotency for `(tenant_id, event_id)`.
- Persist append-only audit records with hash-chain integrity fields.
- Accept and reconcile job and approval lifecycle events.

## What Arbiter does not do (by design)

These are out of scope by responsibility boundary, not missing features.

- It does not execute actions such as sending messages or calling tools.
- It does not generate text itself.
- It does not run job workers.
- It does not provide end-user approval UI.
- It does not manage external product credentials for connectors.

Arbiter is the decision plane. Execution belongs to the execution plane.

## Core guarantees

- Deterministic decision behavior for the same input, policy, and state.
- Explicit fail posture with visible reason codes.
- Idempotent event handling.
- Explainable decision trace in audit records.
- Tamper-evident audit chain (`prev_hash`, `record_hash`).

## API overview (v1)

- `POST /v1/events`
- `POST /v1/generations`
- `POST /v1/job-events`
- `POST /v1/job-cancel`
- `POST /v1/approval-events`
- `POST /v1/action-results`
- `GET /v1/action-results/{tenant_id}/{plan_id}/{action_id}`
- `GET /v1/contracts`
- `GET /v1/healthz`

OpenAPI: `openapi/v1.yaml`

`POST /v1/action-results` request contract (v1):

- required: `v`, `plan_id`, `action_id`, `tenant_id`, `status`, `ts`
- optional: `provider_message_id`, `reason_code`, `error`
- status enum: `succeeded` | `failed` | `skipped`
- idempotency key: `tenant_id + plan_id + action_id`
- conflicting duplicate payload mismatch: returns `409` (`conflict.payload_mismatch`)

Read APIs:

- `GET /v1/jobs/{tenant_id}/{job_id}`
- `GET /v1/approvals/{tenant_id}/{approval_id}`
- `GET /v1/action-results/{tenant_id}/{plan_id}/{action_id}`

## Contracts and versioning

- Active contract set: `contracts/v1/*`
- Contract runtime version: `v=1`
- OpenAPI schema source: `openapi/v1.yaml` references `contracts/v1/*` directly
- `GET /v1/contracts` is generated from a build-time manifest derived from OpenAPI/contracts sources and exposes source hashes
- Compatibility policy: `docs/contract-compatibility-policy.md`

## Storage

Supported stores:

- `memory`
- `sqlite`

Any other `store.kind` fails at startup.
When `store.kind=sqlite`, `store.sqlite_path` is required.

SQLite migration baseline:

- Startup creates missing tables using `CREATE TABLE IF NOT EXISTS`.
- Evolution is additive-first.
- Determinism and idempotency behavior must not change across upgrades.

## Audit integrity

Audit records are append-only and include hash-chain fields.

- `prev_hash`: previous record hash
- `record_hash`: hash of current record seed

Optional immutable mirror sink can be configured via `audit.immutable_mirror_path`.

Verify audit chain:

```bash
arbiter audit-verify --path ./arbiter-audit.jsonl
arbiter audit-verify --path ./arbiter-audit.jsonl --mirror-path ./arbiter-audit-mirror.jsonl
```

## Quick start

Install toolchain:

```bash
mise install
```

Run server:

```bash
mise exec -- cargo run -- serve --config ./config/example-config.yaml
```

Build binary:

```bash
mise run build
./target/release/arbiter serve --config ./config/example-config.yaml
```

## Local quality gates

Run CI-equivalent checks:

```bash
mise run fmt-check
mise run lint
mise run contracts-verify
mise run test
mise run build
mise run ci
```

## Release automation

- Push a SemVer tag (for example, `v1.1.0`) to trigger the release workflow.
- CI validates tag/Cargo/OpenAPI version consistency before publishing.
- GitHub Release notes are generated automatically and include attached binary/checksum artifacts.
- Changelog and release scope: `CHANGELOG.md`, `docs/releases/v1.1.0.md`

## Operations

- SLO draft: `docs/slo.md`
- Runbook: `docs/runbook.md`
- AuthZ resilience policy: `docs/authz-resilience.md`
- State semantics: `docs/state/jobs.md`, `docs/state/approvals.md`, `docs/state/action-results.md`
- Audit mirror semantics: `docs/audit-mirror.md`
- Contracts metadata generation: `docs/contracts-metadata.md`
- Contracts endpoint semantics: `docs/contracts-endpoint.md`
- Release scope (v1.1.0): `docs/releases/v1.1.0.md`

## Design documents

- `docs/architecture-principles.md`
- `docs/decision-log.md`
- `docs/operational-philosophy.md`
- `docs/extensibility-roadmap.md`
- `docs/contracts-intent.md`
- `docs/contract-compatibility-policy.md`
