# AstrBot Pages Observatory Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Repair the ordinary-delivery causal race and ship a development-only, read-only AstrBot Plugin Page that observes committed AstrEmbodiment activity without advancing runtime state or consuming model tokens.

**Execution status (2026-08-29):** Tasks 1–5 are implemented. Native final fix `de5e45a` and Host/Frontend passed independent review; mood retention landed in `ccef619`. Playwright mock bridge passed at desktop and 390 px. Windows wheel build/runtime is PASS; Linux wheel build/static inspection is PASS. Real authenticated AstrBot Page acceptance and Linux runtime smoke remain pending, so Task 6 and the overall release gate are PARTIAL.

**Architecture:** Keep Native/store CAS authoritative. Ordinary delivery may rebuild one never-committed event against a freshly inspected revision; proactive delivery remains ID-addressed dispatch settlement. Add closed committed-only native observation projections, expose them through authenticated GET-only AstrBot Page APIs, and render them in a hash-routed vanilla HTML/CSS/ES-module shell that can grow under the same Page URL.

**Tech Stack:** Rust workspace, PyO3, Python 3.12, AstrBot Plugin Pages v4.24.2+, SQLite, vanilla HTML/CSS/ES modules.

---

### Task 1: Repair stale ordinary delivery settlement

**Files:**
- Modify: `astr_embodiment/coordinator.py`
- Modify: `main.py`
- Test: `tests/test_runtime_integration.py`

- [x] **Step 1: Add one focused failing integration test**

Model `apply_delivery(base=1)` raising `StaleCausalBase`, `inspect()` returning committed revision `2`, and a second call succeeding. Assert the second event preserves event/turn/delivery evidence and only changes `base_revision`. Add one fail-closed case proving a second stale result is not retried a third time.

- [x] **Step 2: Verify RED**

```powershell
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
python -m pytest tests/test_runtime_integration.py -k 'delivery and stale' -q -o cache_dir=G:\AstrEmbodiment\.codex-task-temp\alpha2-pytest-cache
```

Expected: FAIL because `RuntimeCoordinator.apply_delivery()` currently propagates the first stale result and `after_message_sent()` freezes delivery time twice/pops pending before a recoverable commit.

- [x] **Step 3: Implement the bounded repair**

Catch only the bridge `StaleCausalBase` from the first ordinary `DeliveryOutcome`. Inspect the same native scope, validate `bound is True` and `current_revision > original_base`, rebuild the never-committed event with the same frozen evidence and current base, then retry exactly once. Preserve native stale rejection and Store CAS. Do not route proactive dispatch through this path. Freeze `delivered_at_ms` once and retain pending evidence until success or terminal failure handling has recorded it.

- [x] **Step 4: Verify GREEN and compile Python sources**

```powershell
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
python -m pytest tests/test_runtime_integration.py -k 'delivery' -q -o cache_dir=G:\AstrEmbodiment\.codex-task-temp\alpha2-pytest-cache
python -m compileall -q main.py astr_embodiment
```

- [x] **Step 5: Commit (`2aa7366`)**

```powershell
git add astr_embodiment/coordinator.py main.py tests/test_runtime_integration.py
git commit -m "fix: settle ordinary delivery across autonomous commits"
```

### Task 2: Add closed committed-only native observation projections

**Files:**
- Modify: `crates/ae-contracts/src/autonomy.rs`
- Modify: `crates/ae-store/src/autonomy.rs`
- Modify: `crates/ae-runtime/src/autonomy.rs`
- Modify: `crates/ae-pyo3/src/lib.rs`
- Test: focused `ae-store`/`ae-runtime` tests colocated with existing autonomy tests

- [x] **Step 1: Write the minimum RED contracts**

Add tests for `observe_snapshot_v1` and `observe_events_v1` that require `mode=committed_only`, limit `1..=64`, a persona-scoped `(journal_revision,event_id)` cursor, a pinned canonical watermark, and unchanged revision/generation/claim/outbound/budget values before and after reads. Add v5-to-v6 migration and v6 corruption tests proving canonical delta, per-revision manifest and inner-event projection stay complete, including zero-event revisions.

- [x] **Step 2: Verify RED**

```powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\cyberhuman-alpha2-cargo'
cargo test --locked -p ae-store observe_ -q
```

Expected: FAIL because the contracts, schema column, cursor binding, and read projections do not exist.

- [x] **Step 3: Implement contracts, migration, and read transactions**

Add closed serde request/response types without ciphertext, raw targets, prompts, SeedCode, caller incarnation, or message text. Upgrade autonomy DB v5 to v6, add `inner_event.journal_revision` plus a per-revision manifest, backfill from reconstructible journal deltas, and index `(persona_scope,journal_revision,event_id)`. Read snapshot/events in one query-only transaction, pin the high-water mark, bind cursors to the resolved persona, validate at most 65 revisions per page, return at most 64 events, return UTC only, and never invoke advance/gate/claim/settle/recover/rebuild.

- [x] **Step 4: Expose PyO3 functions and verify GREEN**

Register `observe_snapshot_v1` and `observe_events_v1`, then run:

```powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\cyberhuman-alpha2-cargo'
cargo test --locked -p ae-store observe_ -q
cargo check --locked --workspace
```

- [x] **Step 5: Commit (`21bf67b`, hardened through `de5e45a`)**

```powershell
git add crates/ae-contracts crates/ae-store crates/ae-runtime crates/ae-pyo3
git commit -m "feat: expose committed observatory projections"
```

### Task 3: Export native APIs and add the authenticated Page boundary

**Files:**
- Modify: `python/astrembodiment_core/__init__.py`
- Modify: `astr_embodiment/bridge.py`
- Create: `astr_embodiment/observatory.py`
- Modify: `main.py`
- Test: `tests/test_observatory_page_api.py`

- [x] **Step 1: Write focused RED tests**

Test feature-detected registration, lazy `astrbot.api.web` import, unauthenticated/plugin-name mismatch rejection, GET-only behavior, opaque expiring scope handles, limit cap 64, cursor/scope binding, forbidden-field redaction, and zero calls to wake/gate/claim/settle/provider/send methods.

- [x] **Step 2: Verify RED**

```powershell
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
python -m pytest tests/test_observatory_page_api.py -q -o cache_dir=G:\AstrEmbodiment\.codex-task-temp\alpha2-pytest-cache
```

- [x] **Step 3: Implement the Host boundary**

Export the two native functions and bridge wrappers. Add an `ObservatoryProjectionService` that resolves server-issued handles and returns strict DTO allowlists. Register only `/<plugin>/observatory/bootstrap` and `/<plugin>/observatory/events` GET routes when `register_web_api` exists. Require non-empty `request.username`, exact `request.plugin_name`, enabled config, valid handle, and bounded inputs. On older AstrBot, log one Pages-unavailable warning while keeping core startup operational.

- [x] **Step 4: Verify GREEN**

```powershell
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
python -m pytest tests/test_observatory_page_api.py -q -o cache_dir=G:\AstrEmbodiment\.codex-task-temp\alpha2-pytest-cache
python -m compileall -q main.py astr_embodiment python/astrembodiment_core
```

- [x] **Step 5: Commit (`6def8ba`)**

```powershell
git add python/astrembodiment_core/__init__.py astr_embodiment/bridge.py astr_embodiment/observatory.py main.py tests/test_observatory_page_api.py
git commit -m "feat: expose read-only AstrBot observatory API"
```

### Task 4: Build the shared-URL AstrBot Page shell

**Files:**
- Create: `pages/observatory/index.html`
- Create: `pages/observatory/app.js`
- Create: `pages/observatory/style.css`
- Create: `.astrbot-plugin/i18n/zh-CN.json`
- Create: `.astrbot-plugin/i18n/en-US.json`
- Test: `tests/test_release_contracts.py`（资源包契约）
- Browser evidence: Playwright mock bridge（desktop / 390 px）

- [x] **Step 1: Add focused asset/contract checks**

Assert one discoverable `pages/observatory/index.html`, only relative local assets, no CDN/raw fetch/EventSource/POST, use of `window.AstrBotPluginPage`, `bridge.ready()`, `bridge.apiGet()`, theme/locale change handling, hash-route whitelist, hidden-page polling pause, capped in-memory events, and no sensitive identifier in the URL.

- [x] **Step 2: Verify pre-implementation failure**

```powershell
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
python -m pytest tests/test_release_contracts.py -q -o cache_dir=G:\AstrEmbodiment\.codex-task-temp\alpha2-pytest-cache
```

- [x] **Step 3: Implement the accepted visual concept**

Use the saved concept `docs/superpowers/specs/assets/astrbot-pages-observatory-concept.png`. Implement the shared shell and hash routes for overview, timeline, sleep, affect, intentions, causality, and resources. Reuse one snapshot/event store. Poll snapshot/events at most every five seconds, stop while hidden/paused, and use bounded backoff. Mark unavailable evidence explicitly. Keep “状态叙事” off and nonfunctional in alpha2 so observation costs zero tokens.

- [x] **Step 4: Verify focused contracts and mock-bridge browser behavior**

Focused contracts and Playwright mock-bridge runs passed at desktop and 390 px. These runs validate bridge-facing behavior and responsive layout only. Loading through a real authenticated AstrBot >=4.24.2 Plugin Page host remains an explicit Task 6 acceptance item.

- [x] **Step 5: Commit (`c47c511`, polling/mood fixes through `ccef619`)**

```powershell
git add pages/observatory .astrbot-plugin/i18n
git commit -m "feat: add AstrBot mind observatory page"
```

### Task 5: Package and document alpha2

**Files:**
- Modify: `scripts/package_plugin.py`
- Modify: `tests/test_release_contracts.py`
- Modify: `_conf_schema.json`
- Modify: `metadata.yaml`
- Modify: `pyproject.toml`
- Modify: `Cargo.toml` and crate manifests using the workspace version
- Modify: `README.md`
- Modify: `CHANGELOG.md`

- [x] **Step 1: Add RED release assertions**

Require Page assets/i18n, both observation native symbols, Windows/Linux native manifests, no tests/crates/cache/source maps, version `1.1.0-alpha2`, and archive size below 16 MiB.

- [x] **Step 2: Verify RED, then implement packaging/version/docs**

Add controlled `pages/` and `.astrbot-plugin/i18n` inclusion, `observatory_pages_enabled=false`, capability/version documentation, AstrBot Pages minimum `>=4.24.2` while preserving older core compatibility, and GET-only security limitations for AstrBot v4.26.7.

- [x] **Step 3: Verify GREEN**

```powershell
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
python -m pytest tests/test_release_contracts.py -q -o cache_dir=G:\AstrEmbodiment\.codex-task-temp\alpha2-pytest-cache
```

- [x] **Step 4: Commit (`de53300`)**

```powershell
git add scripts/package_plugin.py tests/test_release_contracts.py _conf_schema.json metadata.yaml pyproject.toml Cargo.toml crates README.md CHANGELOG.md
git commit -m "chore: prepare 1.1.0-alpha2 observatory release"
```

### Task 6: Acceptance and universal artifact

**Files:**
- Evidence only under `G:\AstrEmbodiment\.codex-task-temp\cyberhuman-v1.1.0-alpha2\`

- [x] **Step 1: Run bounded compilation and focused contracts**

```powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\cyberhuman-alpha2-cargo'
cargo check --locked --workspace
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
python -m pytest tests/test_runtime_integration.py tests/test_observatory_page_api.py tests/test_release_contracts.py -q -o cache_dir=G:\AstrEmbodiment\.codex-task-temp\alpha2-pytest-cache
```

- [x] **Step 2: Build native wheels and universal ZIP**

Fresh Windows and Linux x86_64 wheels were built and bound into the package manifest. Windows build/runtime is PASS; Linux build/static inspection is PASS, while Linux runtime smoke remains pending.

- [ ] **Step 3: Verify the artifact and real host — PARTIAL**

Archive structure, package checks, both native API markers and mock-bridge page behavior are verified. Real authenticated AstrBot Page discovery/authentication/routing and Linux native runtime smoke remain pending; report `PARTIAL`, not `PASS`, until both are observed.

- [x] **Step 4: Final code review; evidence references remain external**

Do not commit generated wheels, ZIPs, caches, screenshots, tokens, or runtime databases. Record artifact paths/hashes in the final handoff.
