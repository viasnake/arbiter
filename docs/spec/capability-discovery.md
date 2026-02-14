# Capability Discovery (v1.2.0)

Capability discovery is decentralized and read-only.

## Arbiter endpoint

- `GET /v1/contracts`
  - publishes governance view
  - publishes contract and OpenAPI fingerprints

## External component endpoint

- `GET /v1/capabilities` (implemented outside Arbiter)

Expected shape (`ops.capabilities`):

- `component` (`connector|executor`)
- `component_version`
- `contracts_set_sha256_seen`
- `capabilities_sha256`
- `capabilities`
  - `sources`
  - `providers`
  - `action_types`
  - `operations`

## Compatibility rule

An app/supervisor should only enable a component when:

1. `contracts_set_sha256_seen` matches Arbiter `contracts_set_sha256`.
2. Component-advertised capabilities satisfy app requirements.
