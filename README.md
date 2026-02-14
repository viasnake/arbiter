# Arbiter

Arbiter is a deterministic control plane for AI systems.

It does not generate text, execute tools, or run agent loops.
Its purpose is to decide what should happen next, under explicit policy,
with auditable and repeatable behavior.

Job and approval lifecycle endpoints also follow this rule: they only update decision state and emit plans; they never execute external work.

## Why Arbiter exists

Modern AI runtimes mix generation, execution, and policy in one place.
That creates hidden coupling and makes failures hard to explain.

Arbiter separates concerns:

- Accept normalized events.
- Evaluate gate and authorization policy.
- Produce deterministic response plans.
- Persist append-only audit records.
- Guarantee idempotency for `(tenant_id, event_id)`.

This separation makes behavior diagnosable and safer to evolve.

## Scope of v0.0.1

Store support:

- `memory`
- `sqlite`

Any other `store.type` value is rejected at startup. When `store.type=sqlite`, `store.sqlite_path` is required.

SQLite migration policy (v0.x):

- Startup creates missing tables with `CREATE TABLE IF NOT EXISTS`.
- Schema evolution is additive-first; destructive migration is deferred until explicit migration tooling is introduced.
- Upgrades must preserve deterministic behavior and idempotency semantics.

Implemented action types:

- `do_nothing`
- `request_generation`
- `send_message`
- `send_reply`
- `start_agent_job`
- `request_approval`

## Contract versioning policy

Contracts under `contracts/` are the long-lived compatibility boundary.

- Non-breaking additions can be released in minor versions.
- Breaking contract changes require a major version bump.
- Deprecations must be documented before removal.
- v0 still treats breaking changes as exceptional and explicitly documented in changelog and decision log.

## Audit integrity baseline

Audit records include a hash chain (`prev_hash`, `record_hash`) to make tampering detectable.
The chain is append-only and carried in JSONL (and sqlite-backed audit when sqlite store is enabled).

## Quick start

Install toolchain via mise:

```bash
mise install
```

```bash
mise exec -- cargo run -- serve --config ./config/example-config.yaml
```

Build a single binary:

```bash
mise run build
./target/release/arbiter serve --config ./config/example-config.yaml
```

Endpoints:

- `POST /v0/events`
- `POST /v0/generations`
- `POST /v0/job-events`
- `POST /v0/job-cancel`
- `POST /v0/approval-events`
- `POST /v0/action-results`
- `GET /v0/contracts`
- `GET /v0/healthz`

Audit chain verification:

```bash
arbiter audit-verify --path ./arbiter-audit.jsonl
```

## Local verification

Run the same checks as CI:

```bash
make ci
```

Or run each step directly:

```bash
mise run fmt-check
mise run lint
mise run test
mise run build
```

## CI

GitHub Actions runs format, lint, test, and release build on push and pull request.
Workflow file: `.github/workflows/ci.yml`.
Toolchain and task orchestration are managed with `mise.toml`.

## Design documents

The design intent is documented under `docs/`.
The emphasis is on why decisions were made and how to extend safely,
not on implementation internals.

- `docs/architecture-principles.md`
- `docs/decision-log.md`
- `docs/operational-philosophy.md`
- `docs/extensibility-roadmap.md`
- `docs/contracts-intent.md`
- `docs/contract-compatibility-policy.md`
- `docs/authz-resilience.md`
- `docs/slo.md`
- `docs/runbook.md`
