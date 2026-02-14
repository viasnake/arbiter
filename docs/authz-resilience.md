# AuthZ Resilience Policy

## Scope

This policy defines timeout, retry, and circuit-breaker behavior for `authz.mode=external_http`.

## Timeout

- `authz.timeout_ms` sets the HTTP client timeout for each attempt.

## Retry

- `authz.retry_max_attempts` is total attempts (first request + retries).
- `authz.retry_backoff_ms` is fixed delay between attempts.
- Retry is attempted for transport errors, non-success HTTP responses, and response parse errors.
- Contract-invalid responses (`v`, `decision`, `policy_version`) are treated as terminal and are not retried.

## Circuit breaker

- `authz.circuit_breaker_failures` is the consecutive failure threshold.
- `authz.circuit_breaker_open_ms` is the open duration once threshold is reached.
- While open, Arbiter short-circuits external calls with `authz_circuit_open_*` reason codes.
- Any successful external decision closes the circuit and resets failure streak.

## Fail mode interaction

- Final outcome still follows `authz.fail_mode` (`deny`, `allow`, `fallback_builtin`).
- Production recommendation remains fail-closed (`deny`).
