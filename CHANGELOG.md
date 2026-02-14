# Changelog

## 1.2.0

### Breaking

- Reset protocol to provider-agnostic envelope contracts (`ops.*`).
- Removed legacy provider-shaped schemas and endpoints from OpenAPI.
- Introduced governance authority response model on `GET /v1/contracts`.

### Added

- JCS (RFC8785) + sha256 fingerprint utility and fixtures.
- Structured error schema (`ops.errors`) and stable error taxonomy.
- Capability discovery schemas/docs (`ops.capabilities`, docs/spec/*).
- Release intent documentation for v1.2.0 (`docs/releases/v1.2.0.md`).

### Changed

- Event idempotency conflict diagnostics now return `existing_hash` and `incoming_hash`.
- Action-result idempotency uses JCS fingerprints with strict mismatch conflict detection.
- Decision-time model derives `evaluation_time` from `event.occurred_at`.
- Audit hash chaining now includes envelope fingerprints.

### Migration note

v1.2.0 is a protocol reset. Consumers must migrate to the new envelope contracts in `contracts/v1/ops.*.schema.json`.

## 1.1.1

- Legacy patch release notes kept for historical context.

## 1.1.0

- Legacy minor release notes kept for historical context.
