# Product README and Repository Metadata Implementation Plan

> **Execution note:** Implement this plan in the RC1 candidate worktree with one
> writer owning `README.md` and `metadata.yaml`. Keep GitHub About as a separate
> coordinator-owned remote metadata action after local acceptance.

**Goal:** Turn the repository landing page into a product introduction while
preserving exact `1.0.0-rc1` capability and publication boundaries.

**Design source:**
`docs/superpowers/specs/2026-08-22-product-readme-design.md`

**Temporary root:**
`G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\agents\product-readme-terra`

---

## Task 1: Capture the documentation contract

**Files:**

- Modify: `tests/test_release_contracts.py`
- Read: `README.md`
- Read: `metadata.yaml`

1. Add focused assertions that require the product headline, the exact
   `用户话语 → 15 维闭合语义证据 → native semantic commit` loop, an honest local
   RC1 status, the observatory section, and the downstream personality-drift
   boundary.
2. Assert that `metadata.yaml` contains the approved `desc` and `short_desc`
   while keeping `name`, `display_name`, `version`, `repo`, `astrbot_version`,
   platforms, and tags unchanged.
3. Run only the new/affected release-contract tests and record the expected RED
   result before changing product files.

Verification:

```powershell
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
& 'C:\Users\pidan\AppData\Local\Microsoft\WindowsApps\python.exe' -m pytest -q tests/test_release_contracts.py -k 'readme or metadata'
```

## Task 2: Rewrite the product-facing opening and navigation

**Files:**

- Modify: `README.md`

1. Replace the warning-first opening with the exact hybrid hero from the design
   spec: value headline, capability loop, concise product introduction, logo,
   badges, navigation, then the compact RC1 status callout.
2. Add short sections for why the product exists, what works today, and how one
   interaction enters continuity. Put quick start before the architecture deep
   dive.
3. Preserve accurate existing technical material and Mermaid diagrams, but
   remove duplicate or obsolete G0-only claims contradicted by the SPC1 path.
4. Add the observatory behavior: INFO SUCCESS/NOOP, WARNING DEGRADED, all 15
   fxp6 values when available, the four current load dimensions, and the privacy
   exclusions.
5. Keep controlled response and outward personality drift explicitly post-RC1.

## Task 3: Synchronize AstrBot metadata

**Files:**

- Modify: `metadata.yaml`

1. Replace only `desc` and `short_desc` with the exact approved Chinese copy.
2. Verify all other metadata fields are unchanged from the task base commit.

## Task 4: Verify and commit the local documentation

**Files:**

- Verify: `README.md`
- Verify: `metadata.yaml`
- Verify: `tests/test_release_contracts.py`

1. Run the focused test from Task 1 and confirm GREEN.
2. Run the complete release-contract file.
3. Run `python -m compileall -q main.py astr_embodiment python tests scripts`.
4. Check Markdown heading anchors and balanced fenced-code/Mermaid blocks.
5. Inspect `git diff --check`, `git diff --stat`, and staged paths. Stage only
   the three owned files.
6. Commit as:

```text
docs: present AstrEmbodiment as a continuity product
```

Return the RED/GREEN commands, results, commit SHA, and any nonblocking
documentation concern. Do not edit `CHANGELOG.md`, runtime source, version/tag,
release ZIPs, GitHub About, or remote state.

## Task 5: Update and verify GitHub About

**Owner:** coordinator after local review and RC1 acceptance.

1. Read back the current repository identity and description.
2. Set only the description to the exact approved About copy. Leave homepage,
   topics, visibility, and other repository settings unchanged.
3. Read the description back with `gh repo view --json description` and record
   the returned value in G-drive evidence.

