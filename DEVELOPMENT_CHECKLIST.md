# Development Checklist

## First machine

- [ ] Install stable Rust toolchain.
- [ ] `cargo check --workspace`.
- [ ] `maturin develop`.
- [ ] Import `astrembodiment_core` and inspect health.
- [ ] Load the AstrBot plugin locally.
- [ ] Create the first SQLite migration.
- [ ] Implement the vertical slice in G0.

## Before the first neural transition

- [ ] Freeze `CanonicalEvent` v1.
- [ ] Freeze authority matrix v1.
- [ ] Freeze `TransitionReceipt` v1.
- [ ] Implement deterministic fixed-point arithmetic tests.
- [ ] Prove `SELF_ACTION` has zero relation residual authority in code.
- [ ] Implement revision/CAS single writer.

## Before public 1.0.0

- [ ] Complete every item in `docs/engineering/VERIFICATION_GAUNTLET.md`.
- [ ] Build platform wheels.
- [ ] Run 2C2G and 1C1G 24-hour envelopes.
- [ ] Verify cross-envelope digests.
- [ ] Complete license, SBOM, security, release notes, and marketplace metadata.
