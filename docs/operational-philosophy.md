# Operational Philosophy

## Control plane failure posture

Arbiter is intended to fail in a diagnosable way.
A visible no-op (`do_nothing`) is preferred over implicit behavior.

## Authorization and fail mode

External authorization can fail due to transport, timeout, or provider errors.
Fail behavior is explicit and configured with `authz.fail_mode`.

In many environments, fail-closed (`deny`) is a practical default.

When non-closed fail modes are used (`allow`, `fallback_builtin`),
the resulting decision reason must remain explicit in audit records.

## Idempotency as an operational invariant

Retries are normal in distributed systems.
Without idempotency, retries can amplify side effects.

Arbiter uses `(tenant_id, event_id)` as the replay boundary to keep response plans stable.

## Audit as a first-class artifact

Audit logs are operational evidence, not debug convenience.
Append-only output preserves event chronology.
When configured, an immutable mirror sink should receive the same append-only records.

## Observability philosophy

Reason codes are used for denials and no-op outcomes.
Operational teams should be able to answer:

- why action was denied
- where the denial happened
- whether a retry should change outcome
