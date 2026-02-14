# Contract Change Notes

## Boundary

Files under `contracts/` and `openapi/` describe current integration shape.
They can change when needed.

## How changes are handled

- Keep changes explicit and documented.
- Update tests together with contract changes.
- Prefer small, reviewable contract diffs.

## Practical guidance

- If a change affects consumers, document the impact in README and docs.
- If behavior changes, update related tests and examples in the same change.
- If a field or endpoint is removed, keep rationale in commit history or decision log.

## Notes

This project is OSS and also used in personal workflows.
The goal is to keep contracts usable and understandable, without unnecessary process overhead.
