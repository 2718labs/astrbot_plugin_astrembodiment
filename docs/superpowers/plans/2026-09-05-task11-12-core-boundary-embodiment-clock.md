# Task 11/12 Core Boundary and Embodiment Clock Addendum

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `subagent-driven-development` or `executing-plans` and execute only the
> current card from the linked work index.

**Date:** 2026-09-05

**Status:** frozen implementation authority for Tasks 11 and 12

**DESIGN_AUTHORITY_ID:** `AE-T11T12-CORE-BOUNDARY-R2`

The implementation base is the exact commit produced by this design revision.
After this docs-only revision commits, the coordinator must report that commit
SHA to every card owner; no document embeds or predicts its own commit SHA.

## Authority and supersession

This addendum is authoritative for Tasks 11 and 12 of
[the emotion/personality core plan](2026-08-31-emotion-personality-core.md).
It supersedes conflicting implementation details in older plans without
rewriting their history. The old Tasks 13/14 contact-intent projection is
cancelled. Later matrix work must not recreate an outbound consumer.

AstrEmbodiment remains an AstrBot plugin. It owns emotion, personality,
allostasis, sleep, persona-local embodied time, Persona Genesis, deterministic
15D to nine-region to 16K x 8 matrix evolution, inbound semantic appraisal and
the typed outcome of AstrBot's ordinary reply.

It permanently does not own proactive contact intent/chat, externalization,
recipient or target selection, secrets, outbox, dispatch, retry, active
message-consent policy or relation scheduling. It has no AstrCyberHuman
dependency and adds no proactive-send path.

Historical DB rows, saved user settings, target/candidate ciphertext and
DPAPI/key files remain byte-identical and non-executable. The design uses a
permanent v9 overlay, a Store execution fence and Task 12 public-surface removal
rather than rewriting or deleting them.

## Layered work package

Read only the layer required for the current role:

- [Product brief](2026-09-05-task11-12-core-boundary-embodiment-clock/product-brief.md)
  for product direction, scope, risk stops and Done.
- [Coordinator index](2026-09-05-task11-12-core-boundary-embodiment-clock/index.md)
  for shared contracts, DAG, waves, write conflicts and current gate.
- [Core-boundary v9 contract](2026-09-05-task11-12-core-boundary-embodiment-clock/contracts/core-boundary-v9.md)
  for migration, overlay, raw preimage and permanent fence.
- [V9 schema manifest](2026-09-05-task11-12-core-boundary-embodiment-clock/contracts/v9-schema-manifest.md)
  for the sole ordered schema input, canonical renderer and golden digests.
- [Core ingress/delivery contract](2026-09-05-task11-12-core-boundary-embodiment-clock/contracts/core-ingress-delivery.md)
  for typed APIs, durable receipts, replay, locks and hot state.
- [Rolling clock contract](2026-09-05-task11-12-core-boundary-embodiment-clock/contracts/rolling-embodiment-clock.md)
  for inventory, profile, tzdb, sleep/matrix projection and bounded storage.
- [Retired surface/release contract](2026-09-05-task11-12-core-boundary-embodiment-clock/contracts/retired-surface-release.md)
  for unsupported operations, module/UI cleanup and same-SHA artifacts.
- [Task cards](2026-09-05-task11-12-core-boundary-embodiment-clock/tasks/)
  for one-owner exact write scopes and acceptance commands.

## Execution order

The compile-safe sequence is 11A, 11B, 11C, 12A, then 12B. The index is the
only task-state authority. Cards are intentionally serialized where Store,
Runtime, PyO3, bridge or Host files overlap; read-only review may run in
parallel.

Card 11A installs the complete v9 schema once, including every later core
receipt/profile/clock object. Cards 11B and 11C consume that schema and are not
authorized to add, alter, reorder or lazily create a v9 object.

The clock defaults to economical local execution: it performs no Provider,
Token, network, target, secret or send work. Its adaptive intervals are polling
ceilings, not mutation cadences. Background lifetime storage is one
authenticated persona head plus at most 64 exact recent receipts and adds no
row to the existing persona journal or semantic history.

## Release boundary

A source compile, static review, one-platform wheel or ZIP preflight is not
release approval. Completion requires focused positive and negative gates,
clean affected-layer compile/import, byte-preservation evidence, independent
review, and real Windows/Linux wheel imports from one clean source SHA with
identical public API, v9 schema and tzdb identities.
