# SLO Draft (v1.0.0 Target)

## Purpose

Define measurable service-level objectives for Arbiter as a decision control plane.

## SLO Candidates

- Decision latency (`POST /v0/events`): p95 <= 100ms, p99 <= 250ms (single-instance baseline)
- Decision correctness: idempotency replay returns identical `ResponsePlan` for same `(tenant_id, event_id)`
- Audit completeness: every accepted request emits at least one audit record
- Audit integrity: every audit record has non-empty `record_hash`; non-first records include `prev_hash`

## Error budget policy

- SLO violation windows are tracked weekly.
- Repeated violations block non-critical feature work until baseline is restored.

## Notes

These SLOs are a draft for v1.0.0 stabilization.
Threshold tuning should follow production telemetry.
