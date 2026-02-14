# Decision Log

This log captures decisions with rationale, trade-offs, and re-evaluation triggers.

## D-001: Builtin AuthZ mode is allow-all

- Decision: `authz.mode=builtin` always allows.
- Why: focus is on control-plane composition and external RBAC integration.
- Trade-off: builtin mode is not policy-rich.
- Revisit when: builtin RBAC semantics are formally specified.

## D-002: Memory and sqlite are both supported stores

- Decision: both `memory` and `sqlite` are supported.
- Why: memory is lightweight for local use; sqlite adds restart-safe state.
- Trade-off: sqlite introduces migration and operational concerns.
- Revisit when: another store backend is needed.

## D-003: Gate rejects with do_nothing

- Decision: all gate violations emit `do_nothing` with reason code.
- Why: explicit no-op plans preserve deterministic behavior and observability.
- Trade-off: caller must inspect plan, not only HTTP status, for policy result.
- Revisit when: transport-level policy signaling is required.

## D-004: Planner is deterministic and seed-based

- Decision: probabilistic choices derive from `hash(event_id)`.
- Why: preserves reproducibility while allowing policy-controlled variation.
- Trade-off: event_id quality affects distribution quality.
- Revisit when: deterministic bucketing or weighted policy tables are introduced.

## D-005: Append-only audit with hash chain

- Decision: audit records are append-only and include `prev_hash` and `record_hash`.
- Why: simple stream output with tamper-evident linkage.
- Trade-off: integrity is local unless mirrored to another sink.
- Revisit when: stronger external integrity guarantees are needed.
