# API Error Codes

This document defines stable machine-readable error codes used by Arbiter v1 APIs.

## Error envelope

Error responses use this shape:

```json
{
  "error": {
    "code": "<code>",
    "message": "<human readable detail>"
  }
}
```

## Conflict errors (`409`)

- `conflict.payload_mismatch`
  - Same idempotency key was reused with a different payload.
  - Used by event-like ingest endpoints with duplicate detection.
- `conflict.invalid_transition`
  - Lifecycle state transition is not allowed by transition rules.
  - Used by job and approval state ingestion.
- `conflict.duplicate_key`
  - Reserved canonical code for duplicate-key conflicts.
  - Not all endpoints currently emit this code directly.

## Other common codes

- `validation_error` (`400`)
  - Input validation failed or request contract is invalid.
- `not_found` (`404`)
  - Requested state resource does not exist.
- `internal.audit_write_failed` (`500`)
  - Audit append failed and request is fail-closed.
- `internal_error` (`500`)
  - Unexpected internal failure.

## Compatibility policy

- Existing error codes are treated as operator-facing contracts.
- New codes may be added additively.
- Reusing an existing code for a different failure class is not allowed.
