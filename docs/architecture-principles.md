# Architecture Principles

## Intent

Arbiter exists to keep decision-making deterministic and inspectable.
The core architectural choice is to isolate policy judgment from content generation and execution.

## Why deterministic control first

AI systems are often judged after incidents.
If the control path is non-deterministic, root cause analysis becomes speculative.

Determinism is therefore a risk-control requirement, not a convenience.

## Why strict pipeline ordering

The pipeline order is fixed to make behavior predictable and auditable:

1. schema validation
2. idempotency check
3. room state load
4. gate evaluation
5. authorization
6. planner evaluation
7. response plan emit
8. audit persist

Changing order changes semantics and creates hidden policy drift.

## Why single binary

Single-binary operation reduces operational dependency risk for the control plane.
It lowers failure surface during incidents and keeps deployment reproducible.

## What is intentionally excluded

Arbiter intentionally excludes:

- LLM generation
- prompt management
- agent runtime loops
- tool execution

This boundary protects control-plane clarity and limits blast radius.
