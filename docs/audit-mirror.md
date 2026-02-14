# Immutable Mirror Semantics

## Behavior

When `audit.immutable_mirror_path` is configured:

- Arbiter writes each audit record to primary `audit.jsonl_path`.
- Arbiter writes the same serialized record to `immutable_mirror_path`.
- Write order is deterministic: primary first, mirror second.

## Failure policy

- Mirror write errors are fail-closed for request processing.
- The triggering request fails instead of silently accepting divergence.

## Verification

Use CLI verification to validate chain integrity and mirror parity:

```bash
arbiter audit-verify --path ./arbiter-audit.jsonl --mirror-path ./arbiter-audit-mirror.jsonl
```

The verifier reports first divergence line and hash details when mismatch is detected.
