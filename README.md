# Arbiter

[English](README.md) | [日本語](README.ja.md)

Arbiter is a provider-agnostic governance mediator for AI applications.

It unifies contracts, policy visibility, deterministic planning, idempotency safety, and tamper-evident audit.

## What Arbiter is

- A protocol authority exposing stable contracts through `/v1/contracts`.
- A deterministic planner that converts `ops.event` into `ops.plan`.
- A governance enforcer for action type, provider allowlist, and approval defaults.
- An idempotency and conflict-diagnosis boundary for events and action-results.

## What Arbiter is not

- Not a connector runtime.
- Not an executor runtime.
- Not a queue, scheduler, or side-effect dispatcher.
- Not a provider-specific integration bundle.

## API surface (v1.2.0)

- `GET /v1/healthz`
- `GET /v1/contracts`
- `POST /v1/events`
- `POST /v1/approval-events`
- `POST /v1/action-results`

OpenAPI: `openapi/v1.yaml`

## Determinism model

- `plan.decision.evaluation_time` is derived from `event.occurred_at`.
- Decision paths do not use wall-clock time.
- Same stored state + same input event payload => same output plan.

## Capability discovery

- Arbiter publishes governance + contract fingerprints via `GET /v1/contracts`.
- External connector/executor services publish `GET /v1/capabilities` outside Arbiter.
- Compatibility is evaluated by matching `contracts_set_sha256`.

## Sequence (minimal)

```text
Connector --> Arbiter: POST /v1/events (ops.event)
Arbiter --> Connector/App: ops.plan
App/Human --> Arbiter: POST /v1/approval-events (ops.approval_event)
Executor --> Arbiter: POST /v1/action-results (ops.action_result)
```

## Example payloads

Event:

```json
{
  "tenant_id": "tenant-a",
  "event_id": "evt-100",
  "occurred_at": "2026-02-14T00:00:00Z",
  "source": "github",
  "kind": "webhook_received",
  "subject": "issue/42",
  "summary": "new issue opened",
  "payload_ref": "s3://raw/events/evt-100.json",
  "labels": {
    "provider": "generic",
    "action_type": "notify",
    "risk": "low",
    "operation": "emit_notification"
  }
}
```

Plan:

```json
{
  "plan_id": "plan_7a7c07f1914df4f2",
  "tenant_id": "tenant-a",
  "event_id": "evt-100",
  "actions": [
    {
      "action_id": "act_6f39f8f29b0ef55b",
      "type": "notify",
      "provider": "generic",
      "operation": "emit_notification",
      "params": {
        "summary": "new issue opened",
        "payload_ref": "s3://raw/events/evt-100.json"
      },
      "risk": "low",
      "requires_approval": false,
      "idempotency_key": "tenant-a:evt-100:act_6f39f8f29b0ef55b"
    }
  ],
  "approval": {
    "required": false
  },
  "decision": {
    "policy_version": "policy:v1.2.0",
    "evaluation_time": "2026-02-14T00:00:00Z"
  }
}
```

Action-result:

```json
{
  "tenant_id": "tenant-a",
  "plan_id": "plan_7a7c07f1914df4f2",
  "action_id": "act_6f39f8f29b0ef55b",
  "status": "succeeded",
  "occurred_at": "2026-02-14T00:00:10Z",
  "evidence": {
    "external_id": "notif-1"
  }
}
```

## Run

```bash
mise install
mise exec -- cargo run -- serve --config ./config/example-config.yaml
```

## Run with Docker (GHCR)

```bash
docker pull ghcr.io/viasnake/arbiter:v1.2.0
docker run --rm -p 8080:8080 \
  -v "$(pwd)/config/example-config.yaml:/app/config/config.yaml:ro" \
  ghcr.io/viasnake/arbiter:v1.2.0 \
  serve --config /app/config/config.yaml
```

Release tags publish `ghcr.io/viasnake/arbiter:vX.Y.Z` only (no `latest`).

## Schema URL policy

- JSON schema `$id` values are pinned to release-tagged raw GitHub URLs.
- Example: `https://raw.githubusercontent.com/viasnake/arbiter/v1.2.0/contracts/v1/ops.event.schema.json`
- On every release, `$id` values must be updated to the new tag and pass drift-guard tests.

## Verify

```bash
mise run version-check
mise run fmt-check
mise run lint
mise run contracts-verify
mise run test
mise run build
```

## Release version bump

```bash
make version-bump VERSION=1.2.1
mise run ci
```

`version-bump` updates Cargo/OpenAPI/API_VERSION/schema `$id`/README examples together.

## Docs

- `docs/spec/envelopes.md`
- `docs/spec/capability-discovery.md`
- `docs/spec/json-fingerprint.md`
- `docs/spec/governance-view.md`
- `docs/spec/errors.md`
- `docs/releases/v1.2.0.md`
