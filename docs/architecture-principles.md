# Architecture Principles

## Intent

Arbiter exists to keep decision behavior visible and explainable.
The main architectural choice is to separate decision logic from execution.

## Core model

At a high level, Arbiter evaluates an input event and returns a response plan.

`process(Event) -> ResponsePlan`

The plan describes what should happen next. It does not execute the action.

## Pipeline shape

Current processing flow:

1. schema validation
2. idempotency check
3. room state load
4. gate evaluation
5. authorization
6. planner evaluation
7. response plan emit
8. audit persist

Keeping this order stable helps preserve operational predictability.

## Why determinism matters

Incident analysis becomes difficult when control paths drift between retries.
Deterministic decision paths make behavior easier to replay and debug.

## Why single binary

Single-binary operation keeps deployment and rollback simple for a small project.
This reduces operational moving parts in day-to-day use.

## Intentionally out of scope

Arbiter does not aim to provide:

- LLM text generation
- prompt management
- agent runtime loops
- tool execution
- connector-specific adapter logic

Those belong to external components that consume Arbiter plans.
