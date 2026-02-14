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

## D-006: `/v1/contracts` is generated from contracts + OpenAPI

- Decision: contracts metadata is generated from `contracts/v1/*.schema.json` and `openapi/v1.yaml` at build time.
- Why: prevent source drift and ensure hash stability for the same repository state.
- Trade-off: `generated_at` is build-time metadata and can differ across separate builds.
- Revisit when: multi-version contract manifests (v1 + v2) are introduced.

## D-007: action-results are idempotent by `(tenant_id, plan_id, action_id)` and reject mismatches

- Decision: action-result ingest is idempotent by defined key, and same key with payload mismatch returns `409`.
- Why: executor retries must be safe and conflict semantics must remain explicit.
- Trade-off: clients must keep retry payloads byte-equivalent for the same idempotency key.
- Revisit when: partial updates or multi-stage action-result lifecycle is required.

## D-008: state read endpoints reflect persisted event streams

- Decision: `GET /v1/jobs/...` and `GET /v1/approvals/...` reflect latest persisted state from explicit events.
- Why: incident diagnostics require retrievable, deterministic state snapshots.
- Trade-off: no implicit/background transitions; expiration requires explicit event input.
- Revisit when: deterministic clock-driven state derivation is formally defined.

## D-009: v1.2.0 uses envelope-based protocol contracts

- Decision: replace provider-shaped contracts with provider-agnostic event/action/plan envelopes.
- Why: Arbiter must be reusable governance infrastructure, not a provider integration bundle.
- Trade-off: v1.1.x payloads are intentionally incompatible.
- Revisit when: a future major protocol revision is required.

## D-010: fingerprints use JCS (RFC8785) + sha256 only

- Decision: canonicalization for idempotency and capability fingerprints is fixed to JCS + sha256.
- Why: deterministic hash behavior must not depend on object ordering, whitespace, or number formatting variations.
- Trade-off: implementation must preserve strict canonicalization behavior and test fixtures.
- Revisit when: standards evolve and migration path is explicitly designed.

## D-011: decision time is derived from input timestamps

- Decision: plan evaluation time is derived from `event.occurred_at`; wall-clock is excluded from decision paths.
- Why: deterministic replay requires identical outputs for identical stored state and identical input payload.
- Trade-off: time-based runtime heuristics are excluded unless explicitly input-driven.
- Revisit when: deterministic, explicit clock inputs are added as protocol fields.
