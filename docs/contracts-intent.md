# Contracts Intent

## Why contracts exist

Contracts are the long-lived boundary of the control plane.
They are designed to keep producer and consumer responsibilities explicit.

## Event intent

`Event` is normalized input, not raw transport payload.
Its role is to provide a stable decision surface independent of adapters.

## ResponsePlan intent

`ResponsePlan` is the explicit output of arbitration.
It says what should happen next, without performing the action.

## Action taxonomy intent

v1.0.0 keeps action types intentionally small:

- no-op (`do_nothing`)
- request generation
- send message
- send reply

The narrow taxonomy prevents premature coupling to runtime specifics.

## Authorization contract intent

AuthZ request/decision contracts separate policy authority from control-plane logic.
This enables external policy engines without embedding their semantics in Arbiter.

## OpenAPI intent

OpenAPI provides transport discoverability.
Behavioral guarantees come from RFC invariants and contracts, not from transport shape alone.
