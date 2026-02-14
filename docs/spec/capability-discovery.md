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

## JSON examples

Arbiter `/v1/contracts` response (excerpt):

```json
{
  "api_version": "1.2.0",
  "openapi_sha256": "b5adf6e69f1b2fa0d7d31ff6f6c9e6f30d2fd6e6d6d4f0a9e4f2a1c3d0f75a61",
  "contracts_set_sha256": "e2f5a5f0980b5f9ec8a22f4fdf7ca8f3b2bf63188bc4d6322df54beec6aa9c31",
  "governance": {
    "allowed_action_types": ["notify", "write_external", "start_job"],
    "allowed_providers": ["generic", "email"]
  }
}
```

Executor `/v1/capabilities` response:

```json
{
  "component": "executor",
  "component_version": "2026.02.14",
  "contracts_set_sha256_seen": "e2f5a5f0980b5f9ec8a22f4fdf7ca8f3b2bf63188bc4d6322df54beec6aa9c31",
  "capabilities_sha256": "4707fbb15ac9ac1ec31fb084f415f8d73b3600a6ee0f4f77d4bc7de228a3a576",
  "capabilities": {
    "sources": [],
    "providers": ["generic", "email"],
    "action_types": ["notify", "write_external"],
    "operations": ["emit_notification", "create_draft"]
  }
}
```

Connector `/v1/capabilities` response:

```json
{
  "component": "connector",
  "component_version": "2026.02.14",
  "contracts_set_sha256_seen": "e2f5a5f0980b5f9ec8a22f4fdf7ca8f3b2bf63188bc4d6322df54beec6aa9c31",
  "capabilities_sha256": "ed08bc212a830f02b18261dc4ec9693e48ca20a311f266f2fccf4d95f232accc",
  "capabilities": {
    "sources": ["github", "discord"],
    "providers": [],
    "action_types": [],
    "operations": ["webhook_received", "message_received"]
  }
}
```
