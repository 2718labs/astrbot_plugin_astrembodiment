# AstrEmbodiment Core Boundary Work Index

Status: frozen design; implementation not started.

## Shared Contracts

- [Core boundary v9](contracts/core-boundary-v9.md): permanent overlay,
  migration router, raw preimage and downgrade fence.
- [V9 schema manifest](contracts/v9-schema-manifest.md): sole canonical schema
  source, object order, renderer and golden source/output digests.
- [Core ingress and ordinary delivery](contracts/core-ingress-delivery.md):
  typed requests, durable receipts, replay, shared locks and hot state.
- [Rolling embodiment clock](contracts/rolling-embodiment-clock.md): persona
  inventory/profile, vendored time, matrix projection and one-head/64 ledger.
- [Retired surface and release](contracts/retired-surface-release.md): closed
  old-operation manifest, Host/UI/module removal and same-SHA artifacts.
- [Product brief](product-brief.md): user-facing direction and stop conditions.

Contracts are normative. A task card may refine only private implementation
structure; it may not widen the product boundary or weaken a contract.

Controlled revision, 2026-09-08: the coordinator accepted the exact historical
Rust DTO/codec exception in
[retired-surface-release Section 8.1.1](contracts/retired-surface-release.md#811-controlled-task-12a-historical-codec-exception)
after Task 12A execution-boundary review. It supersedes the earlier opaque-only
packaging requirement without restoring submission authority. The docs-only
handoff also records [legacy test migration](tasks/12a-test-migration.md);
retained matrix/perception algorithm coverage must be reconnected, not deleted.

## Tasks

| Card | Status | Owner | Depends on |
| --- | --- | --- | --- |
| [11A](tasks/11a.md) | ready | Store migration owner | none |
| [11B](tasks/11b.md) | pending | Core API owner | 11A |
| [11C](tasks/11c.md) | pending | Host clock owner | 11B |
| [12A](tasks/12a.md) | pending | Boundary removal owner | 11C |
| [12B](tasks/12b.md) | pending | Release provenance owner | 12A |

## DAG and Waves

The implementation DAG is strictly 11A -> 11B -> 11C -> 12A -> 12B.

- Wave 1: 11A only.
- Wave 2: 11B after the v9 schema and fence are compiled and accepted.
- Wave 3: 11C after every new typed method exists across Native and bridge.
- Wave 4: 12A atomically removes the old surface after Host cutover.
- Wave 5: 12B produces cross-platform evidence and the package.

A read-only reviewer may run alongside a writer. No second writer may enter the
same wave.

## Write Conflicts

The sequence is mandatory because 11A and 11B both touch Store registration;
11B and 12A both touch contracts, Runtime, PyO3 and bridge registration; 11C
and 12A both touch main.py and Host bridge surfaces; 12A and 12B both touch
surface/package attestations. Do not merge these cards into parallel writers.

Each owner writes only its card's Write Scope. An extra path requires an index
update and owner handoff before editing.

## Shared Gates

- Exact-v9 reopen is verified before every mutating migration prologue and has
  zero SQL writes and zero total_changes.
- First upgrade is overlay-only; every legacy row and protected file remains
  byte-identical.
- Core inbound persists every closed begin state; Existing never authorizes a
  Provider call.
- Ordinary delivery references durable inbound identity and survives an
  intervening clock commit.
- Clock status/NotDue/Existing are zero-write; background clock lifetime is one
  head plus at most 64 receipts and adds no old-journal rows.
- The retired manifest is rejected before full decode, Runtime core or DB open;
  direct retired symbols are absent.
- Clock execution has zero Provider, Token, network, target, secret and send
  calls.
- Final Windows/Linux wheels import from one clean source SHA with identical
  API, schema and tzdb identities.

## Dispatch

Current wave: 11A.

Next gate: independent review of 11A's migration evidence plus a clean compile
of ae-contracts and ae-store.

The coordinator supplies each owner the exact implementation-base SHA: the
commit containing `DESIGN_AUTHORITY_ID=AE-T11T12-CORE-BOUNDARY-R2`. A card must
stop on any different authority ID or unreported base; the design never cites
its own future commit hash.

On completion, the owner returns exact changed paths, source SHA, commands and
unabridged pass/fail summary. A disconnect, static inspection or partial test is
not completion.
