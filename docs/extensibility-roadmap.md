# Extensibility Roadmap

## Purpose

This roadmap describes extension seams that preserve v1.0.0 invariants.

## Invariants to preserve

- deterministic `process(Event) -> ResponsePlan`
- fixed pipeline ordering
- idempotency for `(tenant_id, event_id)`
- append-only audit semantics

## Planned extension areas

## 1) Job lifecycle actions

Potential additions:

- `start_agent_job`
- job status updates and cancellation paths

Design constraint:
job orchestration should remain outside Arbiter execution responsibilities.

## 2) Approval workflows

Potential additions:

- `request_approval`
- multi-step policy checkpoints

Design constraint:
approval state should be explicit and auditable as policy decisions.

## 3) Audit integrity upgrades

Potential additions:

- hash chain linking audit records
- external immutable sinks

Design constraint:
existing JSONL consumers should remain backward compatible.

## 4) Multi-region and durable state

Potential additions:

- durable idempotency store
- cross-region consistency policies

Design constraint:
deterministic output must hold given equivalent inputs and policy snapshots.
