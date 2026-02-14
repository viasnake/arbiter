# Contract Compatibility Policy (v1.0.0)

## Boundary

Files under `contracts/` define the public compatibility boundary.

## Allowed non-breaking changes

- Add optional fields
- Add new enum variants only when consumers are explicitly version-gated
- Add new endpoints that do not alter existing response semantics

## Breaking changes

The following require a major version bump:

- Remove or rename required fields
- Change field type or meaning
- Remove supported action or status value
- Change deterministic behavior for the same input/policy/state tuple

## Deprecation process

1. Mark as deprecated in docs
2. Keep compatibility for at least one minor release
3. Remove only in next major version with migration note

## Enforcement

- Schema drift tests must pass in CI
- Golden determinism tests must pass in CI
- Breaking contracts must include migration documentation
