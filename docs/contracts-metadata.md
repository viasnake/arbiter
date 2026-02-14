# Contracts Metadata Generation

## Purpose

`GET /v1/contracts` must be derived from repository source-of-truth files to avoid drift.

- Contract schemas: `contracts/v1/*.schema.json`
- OpenAPI file: `openapi/v1.yaml`

## Build-time generation

The `arbiter-contracts` crate generates metadata at build time.

- Enumerates `contracts/v1/*.schema.json`
- Sorts schema paths lexicographically by canonical path string
- Computes per-schema `sha256` from raw file bytes (no JSON normalization)
- Computes `contracts_set_sha256` from canonical concatenation
- Computes `openapi_sha256` from raw `openapi/v1.yaml` bytes
- Emits a generated Rust module consumed by runtime code

## Canonical path format

Schema map keys use OpenAPI-compatible relative paths:

- `../contracts/v1/<name>.schema.json`

This matches `$ref` path style in `openapi/v1.yaml`.

## Canonical set-hash rule

`contracts_set_sha256` is the SHA-256 over this byte sequence:

For each schema entry in sorted canonical-path order:

1. `canonical_path` bytes
2. one `NUL` byte (`0x00`)
3. raw schema file bytes
4. one `NUL` byte (`0x00`)

This rule is deterministic for the same repository state.

## Versioning notes

- v1 metadata generation only targets `contracts/v1/*`.
- Future `v2` should add a separate manifest function and keep v1 behavior stable.
- Existing metadata fields are additive-first; avoid breaking key renames.
