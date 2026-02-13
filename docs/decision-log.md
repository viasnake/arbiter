# Decision Log

This log captures decisions with rationale, trade-offs, and re-evaluation triggers.

## D-001: Builtin AuthZ mode is allow-all in v0.0.1

- Decision: `authz.mode=builtin` always allows.
- Why: v0.0.1 focuses on control-plane composition and external RBAC integration.
- Trade-off: builtin mode is not policy-rich.
- Revisit when: builtin RBAC semantics are formally specified.

## D-002: In-memory store as the default implementation

- Decision: idempotency, room state, and rate counters are stored in memory.
- Why: fastest path to validate invariants and API behavior.
- Trade-off: state is process-local and non-durable.
- Revisit when: deployment requires restart-safe state or multi-instance consistency.

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

## D-005: Append-only JSONL audit sink

- Decision: audit sink writes newline-delimited JSON records in append mode.
- Why: simple, inspectable, and stream-friendly for initial operations.
- Trade-off: no built-in tamper-evident chain in v0.0.1.
- Revisit when: cryptographic integrity guarantees are required.
