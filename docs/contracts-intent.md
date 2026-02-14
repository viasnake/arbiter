# Contracts Intent

## Why contracts exist

Contracts define the current integration shape between Arbiter and its clients.
They make producer and consumer responsibilities explicit.

Contracts may evolve when practical needs change.
The goal is clarity, not rigidity.

## Event intent

`Event` is normalized input, not raw transport payload.
Its role is to provide a consistent decision surface independent of adapters.

## ResponsePlan intent

`ResponsePlan` is the explicit output of arbitration.
It says what should happen next, without performing the action.

## Action taxonomy intent

Action types are intentionally small:

- no-op (`do_nothing`)
- request generation
- send message
- send reply
- start agent job
- request approval

The narrow taxonomy prevents premature coupling to runtime specifics.

## Authorization contract intent

AuthZ request/decision contracts separate policy authority from control-plane logic.
This enables external policy engines without embedding their semantics in Arbiter.

## OpenAPI intent

OpenAPI provides transport discoverability.
Behavioral guarantees come from implementation behavior and tests, not from transport shape alone.
