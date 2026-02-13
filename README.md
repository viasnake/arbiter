# Arbiter

Arbiter is a deterministic control plane for AI systems.

It does not generate text, execute tools, or run agent loops.
Its purpose is to decide what should happen next, under explicit policy,
with auditable and repeatable behavior.

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

Implemented action types:

- `do_nothing`
- `request_generation`
- `send_message`
- `send_reply`

Reserved (not implemented in behavior):

- `start_agent_job`
- `request_approval`

## Quick start

```bash
cargo run -- serve --config ./config/example-config.yaml
```

Endpoints:

- `POST /v0/events`
- `POST /v0/generations`
- `POST /v0/action-results`
- `GET /v0/contracts`
- `GET /v0/healthz`

## Design documents

The design intent is documented under `docs/`.
The emphasis is on why decisions were made and how to extend safely,
not on implementation internals.

- `docs/architecture-principles.md`
- `docs/decision-log.md`
- `docs/operational-philosophy.md`
- `docs/extensibility-roadmap.md`
- `docs/contracts-intent.md`
