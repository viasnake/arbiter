# Envelope Protocol (v1.2.0)

Arbiter v1.2.0 defines a provider-agnostic envelope protocol.

## Event envelope (`ops.event`)

Required fields:

- `tenant_id`
- `event_id`
- `occurred_at` (RFC3339)
- `source`
- `kind`
- `subject`
- `summary`
- `payload_ref`

Optional fields:

- `labels` (`object<string,string>`)
- `actor` (opaque object)
- `context` (`object<string,string>`)

## Action envelope (`ops.action`)

Required fields:

- `action_id`
- `type` (`notify|write_external|start_job`)
- `provider`
- `operation`
- `params` (opaque object)
- `risk` (`low|medium|high`)
- `requires_approval` (boolean)
- `idempotency_key`

## Plan envelope (`ops.plan`)

Required fields:

- `plan_id`
- `tenant_id`
- `event_id`
- `actions`
- `decision.policy_version`
- `decision.evaluation_time` (derived from `event.occurred_at`)

Optional:

- `approval` (`null` or object with `required` and optional `approval_id`)
- `decision.notes`

## Approval event (`ops.approval_event`)

- `tenant_id`
- `approval_id`
- `status` (`requested|approved|denied|canceled`)
- `decided_at`
- `decided_by`
- `reason` (optional)

## Action-result (`ops.action_result`)

- `tenant_id`
- `plan_id`
- `action_id`
- `status` (`succeeded|failed|skipped`)
- `occurred_at`
- `evidence` (opaque object)
