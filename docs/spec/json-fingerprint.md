# JSON Fingerprinting (v1.2.0)

Arbiter uses a single canonical fingerprint rule:

1. Canonicalize JSON using JCS (RFC8785).
2. Compute sha256 on UTF-8 bytes of canonical JSON.
3. Encode digest as lower-case hex.

## Properties

- Key order does not change hash.
- Whitespace does not change hash.
- Equivalent numeric forms hash identically when JCS-canonicalized.
- Unicode strings are normalized per JSON string rules and escaped canonically by JCS.

## Scope

- Event idempotency fingerprints (`POST /v1/events`).
- Action-result idempotency fingerprints (`POST /v1/action-results`).
- Capability discovery fingerprints (`contracts_set_sha256_seen`, `capabilities_sha256`).
- Audit record hashing.
