# Operational Metrics (AS-IS)

This file describes what is currently observed or checked in operation.
It is not a service-level promise.

## Current checkpoints

- Decision endpoint behavior is checked through automated tests on `POST /v1/events`.
- Idempotency behavior is checked by replay tests for `(tenant_id, event_id)`.
- Audit completeness is checked by verifying emitted audit records for accepted requests.
- Audit integrity is checked via `record_hash` / `prev_hash` and `arbiter audit-verify`.

## Current workflow

- Use CI (`fmt-check`, `lint`, `test`, `build`) as baseline quality checks.
- Use runbook steps for incident triage and local diagnosis.

## Notes

Operational thresholds, alert rules, and reporting cadence are project-specific and may change.
