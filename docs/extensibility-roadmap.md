# Extensibility Roadmap

## Purpose

This roadmap describes extension seams while keeping current behavior understandable.

## Current baseline

- deterministic `process(Event) -> ResponsePlan`
- fixed pipeline ordering
- idempotency for `(tenant_id, event_id)`
- append-only audit semantics

## Implemented extension areas

### 1) Job lifecycle actions

Available interfaces:

- `start_agent_job`
- job status updates and cancellation paths

Note:
job orchestration remains outside Arbiter execution responsibilities.

### 2) Approval workflows

Available interfaces:

- `request_approval`

Current behavior supports explicit approval events and timeout/escalation metadata.

Note:
approval state is kept explicit and auditable as policy decisions.

### 3) Audit integrity

Available capabilities:

- hash chain linking audit records
- external immutable sinks

### 4) Durable state

Available capabilities:

- durable idempotency, room, pending, tenant rate, and audit in sqlite

## Future topics (open)

- additional store backends
- stronger audit integrity integration with external systems
- optional policy module expansion for builtin authz
