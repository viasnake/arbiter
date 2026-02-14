# Error Codes (v1.2.0)

Error payload schema: `ops.errors`.

```json
{
  "error": {
    "code": "string",
    "message": "string",
    "details": {}
  }
}
```

## Required stable codes

- `conflict.payload_mismatch`
- `config.invalid_store_kind`
- `request.schema_invalid`
- `policy.provider_not_allowed`
- `policy.action_type_not_allowed`

## Notes

- `details` is optional and used for diagnostics (for example, existing and incoming payload hashes).
- Unknown/temporary server failures should use internal error codes without changing stable public codes.
