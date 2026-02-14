# Event Idempotency Semantics

This document defines idempotency and duplicate diagnostics for `POST /v1/events`.

## Scope

- Endpoint: `POST /v1/events`
- Contract: `contracts/v1/event.schema.json`
- Idempotency key: `(tenant_id, event_id)`

## Canonical Payload

Incoming event payloads are normalized to a canonical JSON string before hashing:

- Object keys are sorted lexicographically.
- Arrays preserve original order.
- Scalar values preserve JSON value semantics.

The service computes:

- `incoming_hash = sha256(canonical_payload_json)`

For new events, the hash is persisted with the event idempotency record.

## Duplicate Handling

When the same `(tenant_id, event_id)` is received again:

1. Existing idempotency record is loaded.
2. Stored payload hash is compared with `incoming_hash`.
3. The endpoint behavior is:
   - Hashes equal: return the existing `ResponsePlan` with `200`.
   - Hashes differ: return `409` with `error.code = conflict.payload_mismatch`.

## Diagnostics

On payload mismatch, the error message includes both hashes:

- `existing_hash=<sha256>`
- `incoming_hash=<sha256>`

This allows operators to distinguish replay of the same input from conflicting reuse of the same idempotency key.

## Notes

- Idempotency is scoped per tenant.
- Determinism guarantees apply to accepted events; conflicting duplicates are rejected explicitly.
