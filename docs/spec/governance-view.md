# Governance View (v1.2.0)

Arbiter publishes governance through `GET /v1/contracts`.

## Response fields

- `api_version`
- `openapi_sha256`
- `contracts_set_sha256`
- `generated_at`
- `schemas` (per-schema sha256 map)
- `governance`
  - `allowed_action_types`
  - `allowed_providers`
  - `approval_policy.required_for_types`
  - `approval_policy.defaults`
  - `max_payload_hints` (optional)
  - `error_codes` (optional)

## Interpretation rules

- Arbiter enforces provider allowlist from config.
- Arbiter enforces fixed action-type universe (`notify`, `write_external`, `start_job`).
- Approval defaults are policy-driven and provider-agnostic.
- Provider and operation strings are opaque identifiers.
