# Emotion Injection Observatory Logging Design

Date: 2026-08-22  
Target: AstrEmbodiment `1.0.0-rc1` candidate  
Status: approved direction, implementation pending

## Context

AstrEmbodiment's SPC1 semantic lane estimates 15 bounded evidence dimensions and
submits them to the native runtime as a perception proposal. The current request
marker retains only the closed `status` and `code`, so operators cannot see the
values used by a successful native commit or diagnose where a failed attempt
stopped.

The observatory log must make every SPC1 outcome visible at ordinary production
log levels. The full vector must not be hidden behind DEBUG. It must also avoid
calling an estimate a committed native state unless a validated native receipt
confirms the commit.

## Goals

- Emit exactly one bounded observatory record for each new SPC1 preflight attempt
  when `observatory_enabled` is enabled.
- Emit all 15 validated semantic evidence dimensions at INFO for a confirmed
  commit.
- Emit INFO for NOOP and WARNING for DEGRADED outcomes, including the complete
  validated vector whenever one exists.
- Distinguish estimated, attempted, committed, deduplicated, and unavailable
  values without exposing content or stable identity tokens.
- Keep the host request marker closed as `{status, code}`.
- Keep observatory failures unable to interrupt the G0 lane or host LLM request.

## Non-goals

- Do not log user text, prompt history, Provider output, or exception text.
- Do not expose SeedCode, scope/event/turn tokens, nonce material, or digests.
- Do not claim all 15 evidence dimensions directly change neural nodes. In the
  RC1 runtime, the load calculation currently consumes only `positive`, `harm`,
  `boundary`, and `epistemic_conflict`; the other dimensions remain validated
  semantic evidence.
- Do not change the Rust ABI, native persistence format, or request marker.

## Selected approach

Use a single allowlisted projection in `main.py` after the coordinator returns
and before `_closed_semantic_outcome()` discards internal details. The coordinator
may return a bounded internal diagnostic envelope containing only validated
numeric evidence, a fixed stage, and a fixed commit state. This envelope is used
for logging and is never attached to the AstrBot request.

This is preferred over logging inside the estimator, where values are not yet
committed, and over widening the request marker, which would weaken the existing
content-free boundary.

## Configuration and levels

The existing `_conf_schema.json` option is the only switch:

- `observatory_enabled=true` (default): emit records.
- `observatory_enabled=false`: emit no observatory record.
- A missing value uses the schema default `true`; a malformed non-boolean value
  fails closed and emits no observatory record.

Log levels are fixed:

- `SUCCESS`: INFO.
- `NOOP`: INFO.
- `DEGRADED`: WARNING.

No SPC1 observatory result is restricted to DEBUG.

## Wire-stable log record

Each record is one compact JSON object prefixed by:

`AstrEmbodiment SPC1 observatory: `

The fixed schema is `astr-embodiment.observatory.semantic-injection.v1`. The
allowlisted fields are:

- `schema`
- `status`
- `code`
- `stage`
- `commit_state`
- `values_state`
- `fxp_scale`
- `dimensions_fxp6`
- `estimator_confidence_fxp6`
- `base_revision`
- `revision`
- `deduplicated`
- `receipt_status`

`fxp_scale` is always `1000000`. Numeric evidence stays as raw integers rather
than floats or arbitrary strings. `dimensions_fxp6`, when available, contains
exactly these keys in this order:

1. `positive`
2. `affiliation`
3. `harm`
4. `boundary`
5. `repair`
6. `repetition`
7. `new_information`
8. `constraint_instability`
9. `epistemic_conflict`
10. `self_responsibility`
11. `other_responsibility`
12. `hostility`
13. `publicness`
14. `engagement`
15. `rejection`

Every dimension is an integer in `0..1000000`; confidence is an integer in
`1..1000000`. JSON serialization is compact, rejects NaN, and never stringifies
untrusted objects.

## Outcome semantics

### Confirmed success

For `SUCCESS / SEMANTIC_COMMITTED`:

- `stage="RECEIPT"`
- `values_state="COMMITTED"`
- `commit_state="CONFIRMED_NEW"` when `deduplicated=false`
- `commit_state="CONFIRMED_EXISTING"` when `deduplicated=true`
- The full vector, confidence, and base revision come from the validated
  proposal.
- Revision, deduplication flag, and receipt status come from the validated
  native result.

Only this outcome describes values as committed.

### NOOP

For `NOOP / EMPTY_REQUEST`:

- `stage="INPUT"`
- `commit_state="NOT_ATTEMPTED"`
- `values_state="UNAVAILABLE"`
- Evidence and revision fields are null.

For `NOOP / ZERO_LOAD`:

- `stage="ESTIMATOR"`
- `commit_state="NOT_ATTEMPTED"`
- `values_state="ESTIMATED_NOT_COMMITTED"`
- The full validated vector and confidence are included at INFO.
- Native revision/result fields are null.

ZERO_LOAD means the four load dimensions are zero; it does not imply that all
other semantic dimensions are zero.

### DEGRADED

The fixed stages are `INPUT`, `ESTIMATOR`, `CURSOR`, `PROPOSAL`, `NATIVE_APPLY`,
and `RECEIPT`.

- Failures before native apply use `commit_state="NOT_ATTEMPTED"`.
- Native apply or receipt failures use `commit_state="UNKNOWN"`, because native
  persistence may have occurred before Python received a valid receipt.
- If a valid estimate already exists, use
  `values_state="ESTIMATED_NOT_CONFIRMED"` and include all 15 values plus
  confidence.
- Otherwise use `values_state="UNAVAILABLE"` and null evidence fields.
- Only the fixed, existing outcome code is logged. Exception and Provider text
  are never logged.

## Privacy boundary

The logger reconstructs the record from validated plain dictionaries and fixed
enums. It must never serialize the full proposal, result, receipt, exception, or
request object. The following are prohibited even if present upstream:

- user prompt, history, system prompt, Provider output, tools, exception text;
- bot/persona/session/relation/event/turn tokens;
- SeedCode, incarnation identifiers, request nonce;
- formula, scope, event, authority, state, or graph digests;
- native residual arrays or arbitrary receipt fields.

## Failure isolation and at-most-once behavior

The observatory helper is `never-raise`: projection or logger failures are
swallowed without changing the G0 result, the pending turn, or the host request.
The existing request-local at-most-once marker remains authoritative. Repeated
hook invocation for the same request must not emit another record.

## TDD and acceptance

Focused RED/GREEN tests will cover:

1. SUCCESS emits INFO with all 15 integer dimensions, confidence, revision, and
   correct new/deduplicated commit state.
2. ZERO_LOAD emits INFO with the full validated estimate and clearly states that
   it was not committed.
3. EMPTY_REQUEST emits an INFO record with unavailable values.
4. Every DEGRADED path emits WARNING with fixed stage/code and correct
   `NOT_ATTEMPTED` or `UNKNOWN` commit state.
5. A valid estimate remains visible on later-stage failure; invalid estimates
   produce null value fields.
6. Explicitly disabled or malformed observatory configuration emits nothing.
7. Sentinels placed in request text, Provider output, exception messages,
   tokens, nonce, and digests never appear in captured logs.
8. Logging/projection failure cannot interrupt the G0 or host LLM lane.
9. Repeated request-hook invocation emits at most one observatory record.

After focused tests pass, run affected Python regressions, Ruff, compileall, real
AstrBot `PluginManager.load()`, release-contract checks, final package smoke, and
the existing RC1 acceptance suite. Because this changes the candidate tree, the
local RC1 tag and ZIP must be regenerated only after current acceptance is green.
