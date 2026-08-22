# AstrEmbodiment Product README and Repository About Design

Date: 2026-08-22  
Target: AstrEmbodiment `1.0.0-rc1` candidate  
Status: approved direction, implementation pending

## Objective

Replace the repository's engineering-disclaimer-first presentation with a
product-first explanation of why AstrEmbodiment exists, what it already does,
and how someone can try it. Preserve every important RC1 limitation, but move
limitations into a compact status callout and a dedicated capability-boundary
section instead of making them the repository's first impression.

The same product language must stay consistent across:

- the GitHub repository About description;
- `metadata.yaml` `desc` and `short_desc`;
- the README hero and product introduction.

## Audience and promise

The primary audience is an AstrBot user or plugin developer who understands the
idea of a persistent Bot persona but does not yet know AstrEmbodiment's internal
architecture.

The product promise is:

> Let a Bot do more than retain experiences: give it a verifiable path for
> continuing to be the same "Ta" across interactions.

The current implementation may claim:

- Rust-native personality-continuity runtime for AstrBot;
- Genesis and persistent SeedCode identity foundation;
- G0 continuity, revision, replay, and delivery boundaries;
- SPC1 current-user-text estimation into 15 validated, closed semantic evidence
  dimensions;
- native semantic proposal commit and a structured local observatory log.

It must not claim:

- a completed controlled-response projection;
- personality drift that already changes the Bot's outward personality;
- a complete emotional-reaction product;
- production readiness, Marketplace availability, or a published GitHub
  Release.

## Selected narrative: value first + capability loop

The README opens with user value and immediately follows it with the implemented
capability loop: user utterance -> 15-dimensional closed semantic evidence ->
native semantic commit. It then explains why that loop matters, proves the
claim with implemented capabilities, and provides a quick start. Technical
architecture, release evidence, and limitations remain available after the
product has been understood.

This hybrid keeps the emotional clarity of a product page while giving a
developer an immediate, falsifiable reason to keep reading. It avoids both a
research-question opening that delays practical value and a pipeline-only
opening that lacks a human reason for the machinery to exist.

## GitHub About

Use this exact description:

> 让你的 Bot 不只记住经历，更能延续“Ta是谁”。AstrEmbodiment 以 Rust 原生运行时承载人格连续性，将用户话语转化为 15 维闭合语义证据并提交原生状态，为受控回应与人格演化提供试验性基础。

The wording leads with value, names the concrete implementation, and describes
controlled response and personality evolution only as the purpose of the
experimental foundation. It does not claim those downstream behaviors are
already complete.

The repository homepage URL remains unchanged unless a real AstrEmbodiment site
already exists. Do not point it to Sylanne's site.

## AstrBot metadata

Use Chinese product copy consistent with the About description:

```yaml
desc: 让你的 Bot 不只记住经历，更能延续“Ta是谁”。AstrEmbodiment 以 Rust 原生运行时承载人格连续性，将用户话语转化为 15 维闭合语义证据并提交原生状态，为受控回应与人格演化提供试验性基础。
short_desc: Rust 原生人格连续性运行时：让用户话语成为可验证、可提交的连续语义证据。
```

Do not change `name`, `display_name`, `version`, `repo`, `astrbot_version`,
platforms, or tags as part of this documentation task.

## README hero

The first screen uses this structure and wording:

```markdown
# 让你的 Bot 不只记住经历，更能延续「Ta是谁」

**用户话语 → 15 维闭合语义证据 → native semantic commit**

AstrEmbodiment 是为 AstrBot 构建的 Rust 原生人格连续性运行时。它为人格建立可验证的 Genesis 起点和可持久的 SeedCode，并让每次进入状态的语义变化都能被验证、提交和追溯。

它正在回答一个比“记住了什么”更难的问题：同一个 Bot，如何在一次次相处之后，仍然能够确认自己是谁、经历了什么，以及哪些变化真正属于自己。
```

Keep the logo and current badges immediately below this copy. Follow them with
a compact status line:

```markdown
> **当前版本：1.0.0-rc1 本地候选。** Genesis、SeedCode、G0 continuity 与 SPC1 semantic commit 已接线并通过本地候选验收；受控回应策略和人格漂移仍是后续能力。尚未创建 GitHub Release，也未上架 AstrBot Marketplace。
```

The status callout is visible but no longer dominates the opening.

## README information architecture

The rewritten README follows this order.

### 1. Hero and navigation

- Product headline, the one-line capability loop, and the introduction above.
- Logo, version/host/platform/license badges.
- Compact links to: product value, current capabilities, quick start, how it
  works, observatory, boundaries, technical architecture, development.

### 2. Why AstrEmbodiment

Explain in plain language:

- Memory answers what happened; personality continuity asks whether those
  experiences belong to the same enduring identity.
- A Persona prompt alone can describe a character but does not provide a
  revisioned, persisted, verifiable identity path.
- AstrEmbodiment establishes that path with a closed identity seed, semantic
  evidence, native state ownership, and replayable receipts.

Use product language first. Define internal terms only when they become useful.

### 3. What works today

Present four concrete capabilities in a compact table:

1. Genesis + SeedCode: establish and retain the same runtime identity.
2. G0 continuity: revision, replay, lifecycle, and delivery-fact boundaries.
3. SPC1 semantic evidence: current user text becomes a validated 15-dimensional
   vector without retaining raw text in native state.
4. Native semantic commit + observatory: commit through the Rust authority and
   expose complete local INFO/WARNING diagnostics without message content.

Every row must state the user-facing result, not only the module name.

### 4. How one interaction enters continuity

Lead with the current implemented flow:

```text
current user text
  -> closed 15-dimensional semantic evidence
  -> Python validation and frozen turn binding
  -> Rust native semantic commit
  -> revisioned receipt and local observatory record
```

The diagram must distinguish the implemented commit path from the future
controlled-response/personality-drift path. Do not label future edges as
current behavior.

### 5. Quick start

Keep installation and configuration concise and actionable:

- supported AstrBot and Python/platform versions;
- local RC1 ZIP installation path;
- first reload and `/ae` health check;
- `native_data_dir`, `assistant_provider_id`, and `observatory_enabled` values;
- where to look for the SPC1 structured log.

Do not bury the first runnable steps under architecture diagrams.

### 6. Observatory: see what was submitted

Document the approved logging behavior:

- SUCCESS and NOOP at INFO; DEGRADED at WARNING;
- all 15 fxp6 dimensions and confidence at ordinary log level whenever a valid
  estimate exists;
- committed, estimated-not-committed, estimated-not-confirmed, and unavailable
  states are distinct;
- only `positive`, `harm`, `boundary`, and `epistemic_conflict` directly
  contribute to the current RC1 load calculation;
- no user message, Provider output, token, nonce, SeedCode, or state digest is
  logged.

Call the 15 values semantic evidence, not 15 neural-node values.

### 7. Current capability boundary

State the boundary in direct, calm language:

- The semantic evidence can be committed to native state today.
- It does not yet drive a complete controlled response strategy.
- Personality drift that changes outward Bot behavior remains post-RC1 work.
- Sylanne's memory, relationship, proactive-chat, TTS, and dashboard features
  are not bundled into AstrEmbodiment.

This section preserves release truth without turning the whole README into a
warning label.

### 8. Technical deep dive

Retain useful existing material, reorganized after the product sections:

- Host/FFI and authority boundary;
- Genesis and SeedCode;
- G0, SPC1, attention/neurofield distinctions;
- revision, replay, storage, delivery settlement;
- observatory and safety;
- integration API and development commands.

Existing accurate Mermaid diagrams may be reused. Remove duplicate paragraphs,
outdated G0-only wording, and statements contradicted by the current SPC1 path.

### 9. Validation, roadmap, and project information

- Summarize current local RC1 acceptance without claiming remote publication.
- Roadmap lists controlled response and personality drift as future work.
- Link changelog, license, security, and contribution information.

## Style rules

- Chinese first; keep established identifiers such as Genesis, SeedCode, G0,
  SPC1, native commit, revision, and fxp6 where they improve precision.
- Prefer short paragraphs and outcome-oriented tables.
- Avoid opening with “源码 Alpha”, “尚未实现”, or “非生产版本”.
- Avoid copying Sylanne's About sentence structure beyond the shared product
  principle of value-first language.
- Do not call the project an emotion engine or claim a living personality.
- Do not erase engineering evidence; relocate it behind the product story.
- Keep the README internally linkable and compatible with GitHub Markdown.

## Verification

Before committing:

- `metadata.yaml` remains parseable and all release fields except `desc` and
  `short_desc` are byte-for-byte unchanged.
- README contains the exact current status and capability boundary.
- README no longer claims SPC1 is entirely future work.
- All internal links and Mermaid fences remain balanced.
- Release-contract tests and real AstrBot plugin loading still pass.

The GitHub About change is a separate remote metadata action. Perform it only
after the local copy is committed and accepted, then read it back with
`gh repo view --json description` and record the exact returned value.
