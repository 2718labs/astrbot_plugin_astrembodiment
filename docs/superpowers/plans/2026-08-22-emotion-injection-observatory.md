# Emotion Injection Observatory Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Emit one ordinary-level, content-free SPC1 observatory log for every semantic preflight outcome, including all 15 validated evidence dimensions whenever they exist.

**Architecture:** `GenesisCoordinator` attaches a bounded, JSON-safe `diagnostic` envelope to its internal preflight outcome at the exact stage where the outcome is known. `AstrEmbodimentPlugin` projects only that envelope into a compact allowlisted JSON log before shrinking the request marker back to `{status, code}`. No Rust ABI, native persistence, or request-marker shape changes.

**Tech Stack:** Python 3.10-compatible source, AstrBot v4.26.7 official logger, pytest, Ruff, native RC1 wheels already accepted on Windows x64 and Linux x86_64.

---

## File map

- Modify `astr_embodiment/coordinator.py`: create safe internal diagnostics and bind each preflight branch to a fixed stage and commit state.
- Modify `main.py`: validate, serialize, and emit the fixed observatory record at INFO or WARNING without leaking arbitrary objects.
- Modify `tests/test_semantic_bridge.py`: TDD coverage for diagnostics at estimator, cursor, native apply, receipt, success, and internal fallback stages.
- Modify `tests/test_runtime_integration.py`: TDD coverage for log levels, complete 15-dimensional output, privacy, configuration, never-raise, and at-most-once behavior.
- Modify `CHANGELOG.md`: record the RC1 observability addition.
- Modify `docs/superpowers/specs/2026-08-22-emotion-injection-observatory-design.md`: retain the self-review correction adding the honest `INTERNAL` stage.

The separately brainstormed product-README rewrite will consume this plan's
logging contract. It owns `README.md` and `metadata.yaml`, so no logging writer
may edit those files concurrently.

All command-created files must stay under:

`G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821`

The implementation worktree is:

`G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\worktrees\rc1-candidate`

### Task 1: Attach bounded diagnostics at the coordinator boundary

**Files:**

- Modify: `astr_embodiment/coordinator.py:33-41,225-253,315-554`
- Test: `tests/test_semantic_bridge.py:243-332,529-761`

- [ ] **Step 1: Extend existing tests with the desired diagnostic contract**

Add `DIMENSION_NAMES`-ordered expectations to the existing success, ZERO_LOAD,
and malformed-estimator tests. The exact assertions are:

```python
assert first["diagnostic"] == {
    "stage": "RECEIPT",
    "commit_state": "CONFIRMED_NEW",
    "values_state": "COMMITTED",
    "dimensions_fxp6": _estimate()["dimensions"],
    "estimator_confidence_fxp6": 800_000,
    "base_revision": 3,
    "revision": 4,
    "deduplicated": False,
    "receipt_status": "committed",
}
```

```python
zero_dimensions = {name: 0 for name in DIMENSION_NAMES}
zero_dimensions["affiliation"] = 1
assert result["status"] == "NOOP"
assert result["code"] == "ZERO_LOAD"
assert result["diagnostic"] == {
    "stage": "ESTIMATOR",
    "commit_state": "NOT_ATTEMPTED",
    "values_state": "ESTIMATED_NOT_COMMITTED",
    "dimensions_fxp6": zero_dimensions,
    "estimator_confidence_fxp6": 1,
    "base_revision": None,
    "revision": None,
    "deduplicated": None,
    "receipt_status": None,
}
```

```python
assert result["status"] == "DEGRADED"
assert result["code"] == "ESTIMATOR_MALFORMED"
assert result["diagnostic"] == {
    "stage": "ESTIMATOR",
    "commit_state": "NOT_ATTEMPTED",
    "values_state": "UNAVAILABLE",
    "dimensions_fxp6": None,
    "estimator_confidence_fxp6": None,
    "base_revision": None,
    "revision": None,
    "deduplicated": None,
    "receipt_status": None,
}
```

Replace exact legacy assertions such as
`result == {"status": "NOOP", "code": "ZERO_LOAD"}` with separate outer
`status`/`code` assertions plus the complete diagnostic assertion. Do this for
every exact result assertion identified by `Select-String`:

```powershell
Select-String -LiteralPath 'tests\test_semantic_bridge.py' -Pattern '== \{"status": "(NOOP|DEGRADED)"'
```

- [ ] **Step 2: Add late-failure and catch-all tests**

Add these tests to `tests/test_semantic_bridge.py`:

```python
def test_native_apply_failure_keeps_validated_values_and_unknown_commit() -> None:
    class FailingBridge:
        def semantic_revision_v1(self, _scope: dict) -> dict:
            return {"schema": "astrembodiment.semantic-revision.v1", "revision": 9}

        def apply_perception_proposal_v1(
            self, _scope: dict, _proposal: str
        ) -> dict:
            raise RuntimeError("NATIVE_EXCEPTION_RAW_SENTINEL")

    async def run() -> dict:
        coordinator = GenesisCoordinator(FailingBridge())  # type: ignore[arg-type]
        return await coordinator.preflight_stimulus(
            _scope(), _turn(), "request", lambda _text: _estimate()
        )

    result = asyncio.run(run())
    assert result["status"] == "DEGRADED"
    assert result["code"] == "NATIVE_ERROR"
    assert result["diagnostic"] == {
        "stage": "NATIVE_APPLY",
        "commit_state": "UNKNOWN",
        "values_state": "ESTIMATED_NOT_CONFIRMED",
        "dimensions_fxp6": _estimate()["dimensions"],
        "estimator_confidence_fxp6": 800_000,
        "base_revision": 9,
        "revision": None,
        "deduplicated": None,
        "receipt_status": None,
    }
    assert "NATIVE_EXCEPTION_RAW_SENTINEL" not in json.dumps(result)


def test_unexpected_preflight_failure_is_internal_and_unknown() -> None:
    async def run() -> dict:
        coordinator = GenesisCoordinator(object())  # type: ignore[arg-type]

        async def explode(**_kwargs: object) -> dict:
            raise RuntimeError("INTERNAL_RAW_SENTINEL")

        coordinator._run_preflight_body = explode  # type: ignore[method-assign]
        return await coordinator.preflight_stimulus(
            _scope(), _turn(), "request", lambda _text: _estimate()
        )

    result = asyncio.run(run())
    assert result["status"] == "DEGRADED"
    assert result["code"] == "NATIVE_ERROR"
    assert result["diagnostic"] == {
        "stage": "INTERNAL",
        "commit_state": "UNKNOWN",
        "values_state": "UNAVAILABLE",
        "dimensions_fxp6": None,
        "estimator_confidence_fxp6": None,
        "base_revision": None,
        "revision": None,
        "deduplicated": None,
        "receipt_status": None,
    }
    assert "INTERNAL_RAW_SENTINEL" not in json.dumps(result)
```

- [ ] **Step 3: Run the new tests and verify RED**

Run:

```powershell
$taskTemp='G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\agents\emotion-task1-red'
New-Item -ItemType Directory -Path $taskTemp -Force | Out-Null
$env:CODEX_TASK_TEMP=$taskTemp
$env:TEMP=Join-Path $taskTemp 'temp'
$env:TMP=$env:TEMP
$env:TMPDIR=$env:TEMP
$env:PYTHONPYCACHEPREFIX=Join-Path $taskTemp 'pycache'
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
New-Item -ItemType Directory -Path $env:TEMP -Force | Out-Null
python -m pytest -q tests/test_semantic_bridge.py -k 'coordinator_preflight or zero_load or malformed_provider or native_apply_failure or unexpected_preflight_failure' -o "cache_dir=$taskTemp\pytest-cache" --basetemp "$taskTemp\pytest-basetemp"
```

Expected: non-zero exit with assertions failing because `diagnostic` is absent;
no production source has changed yet.

- [ ] **Step 4: Implement the diagnostic builders**

Import `DIMENSION_NAMES` beside `SemanticEstimate`, then replace the old two-field
builders with:

```python
    @staticmethod
    def _preflight_diagnostic(
        *,
        stage: str,
        commit_state: str,
        values_state: str,
        estimate: SemanticEstimate | None = None,
        base_revision: int | None = None,
        revision: int | None = None,
        deduplicated: bool | None = None,
        receipt_status: str | None = None,
    ) -> dict[str, Any]:
        dimensions = None
        confidence = None
        if estimate is not None:
            dimensions = {
                name: estimate.dimensions[name] for name in DIMENSION_NAMES
            }
            confidence = estimate.estimator_confidence
        return {
            "stage": stage,
            "commit_state": commit_state,
            "values_state": values_state,
            "dimensions_fxp6": dimensions,
            "estimator_confidence_fxp6": confidence,
            "base_revision": base_revision,
            "revision": revision,
            "deduplicated": deduplicated,
            "receipt_status": receipt_status,
        }

    @classmethod
    def _preflight_failure(
        cls,
        code: str,
        *,
        stage: str = "INTERNAL",
        commit_state: str = "UNKNOWN",
        estimate: SemanticEstimate | None = None,
        base_revision: int | None = None,
    ) -> dict[str, Any]:
        return {
            "status": SEMANTIC_DEGRADED,
            "code": code,
            "diagnostic": cls._preflight_diagnostic(
                stage=stage,
                commit_state=commit_state,
                values_state=(
                    "ESTIMATED_NOT_CONFIRMED"
                    if estimate is not None
                    else "UNAVAILABLE"
                ),
                estimate=estimate,
                base_revision=base_revision,
            ),
        }

    @classmethod
    def _preflight_noop(
        cls,
        code: str,
        *,
        stage: str,
        estimate: SemanticEstimate | None = None,
    ) -> dict[str, Any]:
        return {
            "status": SEMANTIC_NOOP,
            "code": code,
            "diagnostic": cls._preflight_diagnostic(
                stage=stage,
                commit_state="NOT_ATTEMPTED",
                values_state=(
                    "ESTIMATED_NOT_COMMITTED"
                    if estimate is not None
                    else "UNAVAILABLE"
                ),
                estimate=estimate,
            ),
        }
```

- [ ] **Step 5: Bind every normal return site to its exact stage**

Use these exact call shapes:

```python
self._preflight_failure(
    "INVALID_TURN", stage="INPUT", commit_state="NOT_ATTEMPTED"
)
self._preflight_noop("EMPTY_REQUEST", stage="INPUT")
self._preflight_failure(
    code, stage="ESTIMATOR", commit_state="NOT_ATTEMPTED"
)
self._preflight_noop("ZERO_LOAD", stage="ESTIMATOR", estimate=estimate)
self._preflight_failure(
    code,
    stage="CURSOR",
    commit_state="NOT_ATTEMPTED",
    estimate=estimate,
)
self._preflight_failure(
    "INVALID_PROPOSAL",
    stage="PROPOSAL",
    commit_state="NOT_ATTEMPTED",
    estimate=estimate,
)
self._preflight_failure(
    code,
    stage="NATIVE_APPLY",
    commit_state="UNKNOWN",
    estimate=estimate,
    base_revision=proposal["base_revision"],
)
self._preflight_failure(
    "NATIVE_MALFORMED",
    stage="RECEIPT",
    commit_state="UNKNOWN",
    estimate=estimate,
    base_revision=proposal["base_revision"],
)
```

Estimator cancellation uses `ESTIMATOR / NOT_ATTEMPTED`. Shared-task,
serialization, deep-copy, and cache fallbacks retain the builder defaults
`INTERNAL / UNKNOWN`. A native-returned ZERO_LOAD uses the same estimator NOOP
shape and intentionally leaves revision fields null.

After `validate_semantic_result()` succeeds, construct the successful diagnostic
from validated objects only:

```python
        receipt = native_result["receipt"]
        return {
            "status": SEMANTIC_SUCCESS,
            "code": "SEMANTIC_COMMITTED",
            "proposal": proposal,
            "result": copy.deepcopy(native_result),
            "diagnostic": self._preflight_diagnostic(
                stage="RECEIPT",
                commit_state=(
                    "CONFIRMED_EXISTING"
                    if native_result["deduplicated"]
                    else "CONFIRMED_NEW"
                ),
                values_state="COMMITTED",
                estimate=estimate,
                base_revision=proposal["base_revision"],
                revision=native_result["revision"],
                deduplicated=native_result["deduplicated"],
                receipt_status=receipt["status"],
            ),
        }
```

- [ ] **Step 6: Run focused tests and verify GREEN**

Run the Step 3 command with task root `emotion-task1-green`.

Expected: selected tests pass. Then run:

```powershell
python -m pytest -q tests/test_semantic_bridge.py -o "cache_dir=$taskTemp\pytest-cache-all" --basetemp "$taskTemp\pytest-basetemp-all"
```

Expected: the whole semantic bridge test file passes.

- [ ] **Step 7: Diff-check and commit Task 1**

```powershell
git diff --check
git add -- astr_embodiment/coordinator.py tests/test_semantic_bridge.py
git commit -m "feat(runtime): expose closed SPC1 diagnostics"
```

### Task 2: Emit complete INFO/WARNING observatory records

**Files:**

- Modify: `main.py:5-8,57-98,100-134,426-474,699-846`
- Test: `tests/test_runtime_integration.py:1-25,1164-1310`

- [ ] **Step 1: Add a deterministic recording logger and outcome fixture**

Add `import main as main_module` beside the existing main import. Add these test
helpers near the SPC1 fixtures:

```python
class RecordingLogger:
    def __init__(self) -> None:
        self.info_messages: list[str] = []
        self.warning_messages: list[str] = []

    def info(self, template: str, *args: object) -> None:
        self.info_messages.append(template % args)

    def warning(self, template: str, *args: object) -> None:
        self.warning_messages.append(template % args)


def _spc1_dimensions() -> dict[str, int]:
    names = (
        "positive",
        "affiliation",
        "harm",
        "boundary",
        "repair",
        "repetition",
        "new_information",
        "constraint_instability",
        "epistemic_conflict",
        "self_responsibility",
        "other_responsibility",
        "hostility",
        "publicness",
        "engagement",
        "rejection",
    )
    return {name: index * 10_000 for index, name in enumerate(names, start=1)}


def _spc1_success_outcome() -> dict:
    return {
        "status": "SUCCESS",
        "code": "SEMANTIC_COMMITTED",
        "diagnostic": {
            "stage": "RECEIPT",
            "commit_state": "CONFIRMED_NEW",
            "values_state": "COMMITTED",
            "dimensions_fxp6": _spc1_dimensions(),
            "estimator_confidence_fxp6": 900_000,
            "base_revision": 7,
            "revision": 8,
            "deduplicated": False,
            "receipt_status": "committed",
        },
    }
```

- [ ] **Step 2: Add RED tests for complete INFO output and privacy**

```python
def test_spc1_observatory_success_emits_all_dimensions_at_info(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    recorder = RecordingLogger()
    monkeypatch.setattr(main_module, "logger", recorder)
    instance = plugin(FakeConfig(observatory_enabled=True), FakeContext())

    outcome = _spc1_success_outcome()
    instance._emit_semantic_observatory(outcome, {"status": "SUCCESS", "code": "SEMANTIC_COMMITTED"})

    assert recorder.warning_messages == []
    assert len(recorder.info_messages) == 1
    prefix = "AstrEmbodiment SPC1 observatory: "
    assert recorder.info_messages[0].startswith(prefix)
    record = json.loads(recorder.info_messages[0][len(prefix):])
    assert record["schema"] == "astr-embodiment.observatory.semantic-injection.v1"
    assert record["status"] == "SUCCESS"
    assert record["code"] == "SEMANTIC_COMMITTED"
    assert record["fxp_scale"] == 1_000_000
    assert record["dimensions_fxp6"] == _spc1_dimensions()
    assert len(record["dimensions_fxp6"]) == 15
    assert record["estimator_confidence_fxp6"] == 900_000
    assert record["revision"] == 8


def test_spc1_observatory_degraded_warns_without_echoing_raw_fields(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    recorder = RecordingLogger()
    monkeypatch.setattr(main_module, "logger", recorder)
    instance = plugin(FakeConfig(observatory_enabled=True), FakeContext())
    raw = {
        "status": "DEGRADED",
        "code": "NATIVE_ERROR",
        "diagnostic": {
            "stage": "NATIVE_APPLY",
            "commit_state": "UNKNOWN",
            "values_state": "ESTIMATED_NOT_CONFIRMED",
            "dimensions_fxp6": _spc1_dimensions(),
            "estimator_confidence_fxp6": 900_000,
            "base_revision": 7,
            "revision": None,
            "deduplicated": None,
            "receipt_status": None,
        },
        "request": "USER_RAW_SENTINEL",
        "exception": "EXCEPTION_RAW_SENTINEL",
        "scope_digest": "DIGEST_RAW_SENTINEL",
    }

    instance._emit_semantic_observatory(raw, {"status": "DEGRADED", "code": "NATIVE_ERROR"})

    assert recorder.info_messages == []
    assert len(recorder.warning_messages) == 1
    encoded = recorder.warning_messages[0]
    assert "USER_RAW_SENTINEL" not in encoded
    assert "EXCEPTION_RAW_SENTINEL" not in encoded
    assert "DIGEST_RAW_SENTINEL" not in encoded
    record = json.loads(encoded.split(": ", 1)[1])
    assert record["status"] == "DEGRADED"
    assert record["stage"] == "NATIVE_APPLY"
    assert record["commit_state"] == "UNKNOWN"
    assert record["dimensions_fxp6"] == _spc1_dimensions()
```

- [ ] **Step 3: Add RED tests for configuration, malformed diagnostics, never-raise, and at-most-once**

Add parameterized configuration coverage:

```python
@pytest.mark.parametrize("configured", [False, "true", 1, None])
def test_spc1_observatory_disabled_or_malformed_config_emits_nothing(
    monkeypatch: pytest.MonkeyPatch, configured: object
) -> None:
    recorder = RecordingLogger()
    monkeypatch.setattr(main_module, "logger", recorder)
    instance = plugin(FakeConfig(observatory_enabled=configured), FakeContext())
    instance._emit_semantic_observatory(
        _spc1_success_outcome(),
        {"status": "SUCCESS", "code": "SEMANTIC_COMMITTED"},
    )
    assert recorder.info_messages == []
    assert recorder.warning_messages == []
```

Add a logger that raises and prove `_emit_semantic_observatory()` returns without
raising:

```python
class RaisingLogger:
    def info(self, _template: str, *_args: object) -> None:
        raise RuntimeError("LOGGER_RAW_SENTINEL")

    def warning(self, _template: str, *_args: object) -> None:
        raise RuntimeError("LOGGER_RAW_SENTINEL")


def test_spc1_observatory_logger_failure_never_raises(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(main_module, "logger", RaisingLogger())
    instance = plugin(FakeConfig(observatory_enabled=True), FakeContext())
    instance._emit_semantic_observatory(
        _spc1_success_outcome(),
        {"status": "SUCCESS", "code": "SEMANTIC_COMMITTED"},
    )
```

Extend the existing repeated-hook test to monkeypatch `main_module.logger`, make
its coordinator stub return `_spc1_success_outcome()`, call the hook twice, and
assert `len(recorder.info_messages) == 1` while the request marker is still only:

```python
{"status": "SUCCESS", "code": "SEMANTIC_COMMITTED"}
```

- [ ] **Step 4: Run logging tests and verify RED**

```powershell
$taskTemp='G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\agents\emotion-task2-red'
New-Item -ItemType Directory -Path $taskTemp -Force | Out-Null
$env:CODEX_TASK_TEMP=$taskTemp
$env:TEMP=Join-Path $taskTemp 'temp'
$env:TMP=$env:TEMP
$env:TMPDIR=$env:TEMP
$env:PYTHONPYCACHEPREFIX=Join-Path $taskTemp 'pycache'
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
New-Item -ItemType Directory -Path $env:TEMP -Force | Out-Null
python -m pytest -q tests/test_runtime_integration.py -k 'spc1_observatory or spc1_repeated_hook' -o "cache_dir=$taskTemp\pytest-cache" --basetemp "$taskTemp\pytest-basetemp"
```

Expected: non-zero exit because `_emit_semantic_observatory` does not exist and
the hook emits no observatory record.

- [ ] **Step 5: Implement the allowlisted record projection**

Add `import json`, import `DIMENSION_NAMES` and `FXP6_SCALE` in both package and
direct-import branches, and add fixed constants:

```python
_SPC1_OBSERVATORY_PREFIX = "AstrEmbodiment SPC1 observatory: "
_SPC1_OBSERVATORY_SCHEMA = "astr-embodiment.observatory.semantic-injection.v1"
_SPC1_DIAGNOSTIC_FIELDS = {
    "stage",
    "commit_state",
    "values_state",
    "dimensions_fxp6",
    "estimator_confidence_fxp6",
    "base_revision",
    "revision",
    "deduplicated",
    "receipt_status",
}
_SPC1_STAGES = {
    "INPUT",
    "ESTIMATOR",
    "CURSOR",
    "PROPOSAL",
    "NATIVE_APPLY",
    "RECEIPT",
    "INTERNAL",
}
_SPC1_COMMIT_STATES = {
    "NOT_ATTEMPTED",
    "UNKNOWN",
    "CONFIRMED_NEW",
    "CONFIRMED_EXISTING",
}
_SPC1_VALUES_STATES = {
    "UNAVAILABLE",
    "ESTIMATED_NOT_COMMITTED",
    "ESTIMATED_NOT_CONFIRMED",
    "COMMITTED",
}
```

Add these methods beside `_closed_semantic_outcome()`:

```python
    def _observatory_enabled(self) -> bool:
        value = self._config_values.get("observatory_enabled", True)
        return type(value) is bool and value

    @staticmethod
    def _fallback_semantic_observatory_record() -> dict[str, Any]:
        return {
            "schema": _SPC1_OBSERVATORY_SCHEMA,
            "status": "DEGRADED",
            "code": "NATIVE_MALFORMED",
            "stage": "INTERNAL",
            "commit_state": "UNKNOWN",
            "values_state": "UNAVAILABLE",
            "fxp_scale": FXP6_SCALE,
            "dimensions_fxp6": None,
            "estimator_confidence_fxp6": None,
            "base_revision": None,
            "revision": None,
            "deduplicated": None,
            "receipt_status": None,
        }

    @classmethod
    def _semantic_observatory_record(
        cls, raw_outcome: Any, closed_outcome: dict[str, str]
    ) -> dict[str, Any]:
        try:
            if type(raw_outcome) is not dict or type(closed_outcome) is not dict:
                raise TypeError
            diagnostic = raw_outcome.get("diagnostic")
            if type(diagnostic) is not dict or set(diagnostic) != _SPC1_DIAGNOSTIC_FIELDS:
                raise ValueError
            stage = diagnostic.get("stage")
            commit_state = diagnostic.get("commit_state")
            values_state = diagnostic.get("values_state")
            if type(stage) is not str or stage not in _SPC1_STAGES:
                raise ValueError
            if type(commit_state) is not str or commit_state not in _SPC1_COMMIT_STATES:
                raise ValueError
            if type(values_state) is not str or values_state not in _SPC1_VALUES_STATES:
                raise ValueError

            dimensions = diagnostic.get("dimensions_fxp6")
            confidence = diagnostic.get("estimator_confidence_fxp6")
            if values_state == "UNAVAILABLE":
                if dimensions is not None or confidence is not None:
                    raise ValueError
                closed_dimensions = None
            else:
                if type(dimensions) is not dict or set(dimensions) != set(DIMENSION_NAMES):
                    raise ValueError
                closed_dimensions = {}
                for name in DIMENSION_NAMES:
                    value = dimensions.get(name)
                    if type(value) is not int or not 0 <= value <= FXP6_SCALE:
                        raise ValueError
                    closed_dimensions[name] = value
                if type(confidence) is not int or not 1 <= confidence <= FXP6_SCALE:
                    raise ValueError

            base_revision = diagnostic.get("base_revision")
            revision = diagnostic.get("revision")
            deduplicated = diagnostic.get("deduplicated")
            receipt_status = diagnostic.get("receipt_status")
            if base_revision is not None and (
                type(base_revision) is not int or base_revision < 0
            ):
                raise ValueError
            if revision is not None and (type(revision) is not int or revision < 0):
                raise ValueError
            if deduplicated is not None and type(deduplicated) is not bool:
                raise ValueError
            if receipt_status not in {None, "committed"}:
                raise ValueError

            status = closed_outcome.get("status")
            code = closed_outcome.get("code")
            if type(status) is not str or type(code) is not str:
                raise ValueError
            return {
                "schema": _SPC1_OBSERVATORY_SCHEMA,
                "status": status,
                "code": code,
                "stage": stage,
                "commit_state": commit_state,
                "values_state": values_state,
                "fxp_scale": FXP6_SCALE,
                "dimensions_fxp6": closed_dimensions,
                "estimator_confidence_fxp6": confidence,
                "base_revision": base_revision,
                "revision": revision,
                "deduplicated": deduplicated,
                "receipt_status": receipt_status,
            }
        except BaseException:
            return cls._fallback_semantic_observatory_record()

    def _emit_semantic_observatory(
        self, raw_outcome: Any, closed_outcome: dict[str, str]
    ) -> None:
        try:
            if not self._observatory_enabled():
                return
            record = self._semantic_observatory_record(raw_outcome, closed_outcome)
            encoded = json.dumps(
                record,
                ensure_ascii=False,
                separators=(",", ":"),
                allow_nan=False,
            )
            if record["status"] == "DEGRADED":
                logger.warning("%s%s", _SPC1_OBSERVATORY_PREFIX, encoded)
            else:
                logger.info("%s%s", _SPC1_OBSERVATORY_PREFIX, encoded)
        except BaseException:
            return
```

- [ ] **Step 6: Wire exactly one observer call into the hook**

Initialize `raw_outcome: Any = None` before constructing `FrozenTurn`. Preserve
the current coordinator call and closed marker. After all SPC1 branches have
assigned `outcome`, but before the final request-marker `setattr`, call:

```python
self._emit_semantic_observatory(raw_outcome, outcome)
```

When the coordinator throws, keep `raw_outcome = None` and the existing fixed
`{"status": "DEGRADED", "code": "NATIVE_ERROR"}` outcome; the projection then
emits the fixed INTERNAL fallback. Do not emit in `_spc1_estimate()`, and do not
attach `diagnostic`, `proposal`, or `result` to the request.

- [ ] **Step 7: Run focused tests and verify GREEN**

Run the Step 4 command with task root `emotion-task2-green`, then run:

```powershell
python -m pytest -q tests/test_runtime_integration.py -o "cache_dir=$taskTemp\pytest-cache-all" --basetemp "$taskTemp\pytest-basetemp-all"
```

Expected: the focused logger tests and all runtime integration tests pass.

- [ ] **Step 8: Diff-check and commit Task 2**

```powershell
git diff --check
git add -- main.py tests/test_runtime_integration.py
git commit -m "feat(plugin): log complete SPC1 outcomes"
```

### Task 3: Record the local diagnostic surface in release history

**Files:**

- Modify: `CHANGELOG.md:21-36`
- Modify: `docs/superpowers/specs/2026-08-22-emotion-injection-observatory-design.md`

- [ ] **Step 1: Update the RC1 changelog**

Insert under the existing `1.0.0-rc1` entry:

```markdown
### 诊断

- `observatory_enabled=true` 时，SPC1 成功提交会以 INFO 单行完整回显 15 维 fxp6 语义证据、置信度、revision 和幂等状态；ZERO_LOAD 仍明确标为未提交。
- SPC1 NOOP 使用 INFO，降级或失败使用 WARNING，并以固定 stage、code 和 commit state 区分“未尝试”“提交结果未知”和“已确认”；日志不包含消息正文、Provider 原文、token、nonce 或 digest。
```

- [ ] **Step 2: Check docs and commit**

```powershell
git diff --check
Select-String -LiteralPath 'CHANGELOG.md' -Pattern '15 维|WARNING|observatory_enabled'
git add -- CHANGELOG.md docs/superpowers/specs/2026-08-22-emotion-injection-observatory-design.md
git commit -m "docs: describe SPC1 observatory records"
```

### Task 4: Re-accept and re-finalize local RC1

**Files and artifacts:**

- Verify all tracked changes in the candidate worktree.
- Create acceptance evidence only below `G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821`.
- Regenerate `G:\AstrEmbodiment\release\astrbot_plugin_astrembodiment-1.0.0-rc1-win_linux_x86_64.zip` only after all current gates pass.
- Move the local annotated `v1.0.0-rc1` tag only after all current gates pass; do not push it.

- [ ] **Step 1: Run current Python and static acceptance**

```powershell
$taskTemp='G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\acceptance\emotion-observatory-final'
New-Item -ItemType Directory -Path $taskTemp -Force | Out-Null
$env:CODEX_TASK_TEMP=$taskTemp
$env:TEMP=Join-Path $taskTemp 'temp'
$env:TMP=$env:TEMP
$env:TMPDIR=$env:TEMP
$env:PYTHONPYCACHEPREFIX=Join-Path $taskTemp 'pycache'
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
New-Item -ItemType Directory -Path $env:TEMP -Force | Out-Null
python -m pytest --collect-only -q tests -o "cache_dir=$taskTemp\pytest-cache-collect" --basetemp "$taskTemp\pytest-basetemp-collect"
python -m pytest -q tests -o "cache_dir=$taskTemp\pytest-cache-full" --basetemp "$taskTemp\pytest-basetemp-full"
python -m pytest -q tests/test_release_contracts.py -o "cache_dir=$taskTemp\pytest-cache-release" --basetemp "$taskTemp\pytest-basetemp-release"
python -m ruff check main.py astr_embodiment tests
python -m ruff format --check main.py astr_embodiment tests
python -m compileall -q main.py astr_embodiment python tests scripts
git diff --check
git status --short --branch
```

Expected: collection and every test/check command exit 0. Third-party `jieba`
warnings may remain non-blocking. No Rust rebuild is required because this change
does not touch Rust source, Cargo metadata, native ABI, or accepted wheel bytes.

- [ ] **Step 2: Package a fresh candidate with the accepted native wheels**

```powershell
$candidateZip=Join-Path $taskTemp 'astrbot_plugin_astrembodiment-1.0.0-rc1-win_linux_x86_64.zip'
$winWheel='G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\acceptance\native-builds\windows-6110b0e\dist\astrembodiment_core-1.0.0rc1-cp312-abi3-win_amd64.whl'
$linuxWheel='G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\acceptance\native-builds\linux-6110b0e\dist\astrembodiment_core-1.0.0rc1-cp312-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64.whl'
python scripts/package_plugin.py --output $candidateZip --native-wheel $winWheel --native-wheel $linuxWheel
Get-Item -LiteralPath $candidateZip | Select-Object FullName,Length
Get-FileHash -Algorithm SHA256 -LiteralPath $candidateZip
```

Expected: package exits 0 and ZIP size is below 16 MiB.

- [ ] **Step 3: Run a fresh AstrBot 4.26.7 PluginManager lifecycle**

Create a new host root and install the candidate archive without reusing the old
host state:

```powershell
$short=(git rev-parse --short=7 HEAD).Trim()
$hostRoot=Join-Path $taskTemp "host-4267-$short"
if (-not $hostRoot.StartsWith($taskTemp, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw 'host root escaped task temp'
}
$pluginRoot=Join-Path $hostRoot 'data\plugins\astrbot_plugin_astrembodiment'
$configRoot=Join-Path $hostRoot 'data\config'
$runtimeRoot=Join-Path $hostRoot 'runtime'
New-Item -ItemType Directory -Path $pluginRoot,$configRoot,$runtimeRoot,(Join-Path $hostRoot 'reserved'),(Join-Path $hostRoot 'tmp') -Force | Out-Null
New-Item -ItemType File -Path (Join-Path $hostRoot 'data\__init__.py'),(Join-Path $hostRoot 'data\plugins\__init__.py') -Force | Out-Null
Expand-Archive -LiteralPath $candidateZip -DestinationPath $pluginRoot
Copy-Item -LiteralPath 'G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\acceptance\host-4267-6110b0e\astrbot_plugin_manager_smoke.py' -Destination (Join-Path $hostRoot 'astrbot_plugin_manager_smoke.py')
$hostConfig=[ordered]@{
    runtime_envelope='auto'
    native_data_dir=$runtimeRoot
    observatory_enabled=$true
    proactive_enabled=$false
    model_settings=[ordered]@{assistant_provider_id=''}
    seed_code=''
}
$hostConfig | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $configRoot 'astrbot_plugin_astrembodiment_config.json') -Encoding utf8NoBOM
'{}' | Set-Content -LiteralPath (Join-Path $hostRoot 'data\cmd_config.json') -Encoding utf8NoBOM
```

Use the existing AstrBot environment:

```powershell
$hostPython='G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\agents\astrbot-4267-host\venv-4267\Scripts\python.exe'
$env:TEMP=Join-Path $hostRoot 'tmp'
$env:TMP=$env:TEMP
$env:TMPDIR=$env:TEMP
$env:PYTHONPYCACHEPREFIX=Join-Path $hostRoot 'pycache'
& $hostPython (Join-Path $hostRoot 'astrbot_plugin_manager_smoke.py')
```

Expected: AstrBot HEAD
`fed29848ca0b3912ab6a8200a10cd0f2cb080f85`, plugin/native version
`1.0.0-rc1`, `g0-ready`, 16,384 neurons, both semantic exports callable,
GENESIS_REQUIRED on the unbound scope, SQLite created, and clean terminate.

- [ ] **Step 4: Run the Linux final-ZIP smoke**

Create `$taskTemp\linux-smoke.py` with this exact content:

```python
from __future__ import annotations

import hashlib
import json
import os
import sys
from pathlib import Path


root = Path(os.environ["AE_LINUX_ARCHIVE_ROOT"])
sys.path.insert(0, str(root))

import astrembodiment_core as native  # noqa: E402


assert native.version() == "1.0.0-rc1"
assert callable(native.semantic_revision_v1)
assert callable(native.apply_perception_proposal_v1)
manifest_path = root / "astrembodiment_core" / "_bundled" / "manifest.json"
manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
entry = manifest["platforms"]["linux"]
native_path = manifest_path.parent / entry["build_id"] / entry["filename"]
assert hashlib.sha256(native_path.read_bytes()).hexdigest() == entry["build_id"]

state_path = Path(os.environ["AE_LINUX_STATE_PATH"])
state_path.parent.mkdir(parents=True, exist_ok=True)
health = json.loads(native.open(str(state_path)))
assert health["status"] == "g0-ready"
assert health["neuron_slots"] == 16_384
scope = json.dumps(
    {
        "bot_token": "11" * 16,
        "persona_token": "22" * 16,
        "relation_token": None,
        "session_token": "33" * 16,
    },
    separators=(",", ":"),
)
try:
    native.semantic_revision_v1(scope)
except native.NativeCoreError as exc:
    assert str(exc).startswith("GENESIS_REQUIRED::")
else:
    raise AssertionError("unbound semantic scope must require Genesis")
native.flush_and_close()
assert state_path.is_file()
print(
    json.dumps(
        {
            "version": native.version(),
            "health": health,
            "manifest_build_id": entry["build_id"],
            "native_sha256": hashlib.sha256(native_path.read_bytes()).hexdigest(),
            "sqlite_size": state_path.stat().st_size,
        },
        sort_keys=True,
    )
)
```

Extract and execute it with the existing managed CPython 3.12 runtime:

```powershell
$linuxRoot=Join-Path $taskTemp "linux-$short"
if (-not $linuxRoot.StartsWith($taskTemp, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw 'linux root escaped task temp'
}
New-Item -ItemType Directory -Path $linuxRoot,(Join-Path $linuxRoot 'home'),(Join-Path $linuxRoot 'tmp'),(Join-Path $linuxRoot 'pycache') -Force | Out-Null
$candidateZipWsl=(wsl.exe -d AstrEmbodimentVerify -- wslpath -a $candidateZip).Trim()
$linuxRootWsl=(wsl.exe -d AstrEmbodimentVerify -- wslpath -a $linuxRoot).Trim()
$smokeWsl=(wsl.exe -d AstrEmbodimentVerify -- wslpath -a (Join-Path $taskTemp 'linux-smoke.py')).Trim()
$linuxPython='/mnt/g/AstrEmbodiment/.codex-task-temp/ae-rc1-takeover-20260821/agents/linux-runtime-final/venv/bin/python'
wsl.exe -d AstrEmbodimentVerify -- env HOME="$linuxRootWsl/home" XDG_CACHE_HOME="$linuxRootWsl/xdg-cache" PYTHONPYCACHEPREFIX="$linuxRootWsl/pycache" TMPDIR="$linuxRootWsl/tmp" $linuxPython -m zipfile -e $candidateZipWsl "$linuxRootWsl/archive"
wsl.exe -d AstrEmbodimentVerify -- env HOME="$linuxRootWsl/home" XDG_CACHE_HOME="$linuxRootWsl/xdg-cache" PYTHONPYCACHEPREFIX="$linuxRootWsl/pycache" TMPDIR="$linuxRootWsl/tmp" AE_LINUX_ARCHIVE_ROOT="$linuxRootWsl/archive" AE_LINUX_STATE_PATH="$linuxRootWsl/state/astrembodiment.sqlite3" $linuxPython $smokeWsl
```

Expected: Linux process exits 0. Record distro, glibc, Python version, manifest
build ID, native SHA-256, and final SQLite size in the acceptance evidence.

- [ ] **Step 5: Preserve the prior artifact, install the accepted ZIP, and retag locally**

Resolve and verify every path before copying. Preserve the old ZIP and checksum
under `$taskTemp\artifact-history\6110b0e\release`; do not delete either file.
Then copy the accepted candidate ZIP to the fixed release path and write its new
SHA-256 checksum. Finally record the old tag object/target, recreate only the
local annotated tag at current HEAD, and verify it:

```powershell
$head=(git rev-parse HEAD).Trim()
$tree=(git rev-parse HEAD^{tree}).Trim()
$oldTagObject=(git rev-parse v1.0.0-rc1^{tag}).Trim()
$oldTagTarget=(git rev-parse v1.0.0-rc1^{}).Trim()
git tag -d v1.0.0-rc1
git tag -a v1.0.0-rc1 -m 'AstrEmbodiment 1.0.0-rc1' $head
$newTagObject=(git rev-parse v1.0.0-rc1^{tag}).Trim()
$newTagTarget=(git rev-parse v1.0.0-rc1^{}).Trim()
[pscustomobject]@{
    Head=$head
    Tree=$tree
    OldTagObject=$oldTagObject
    OldTagTarget=$oldTagTarget
    NewTagObject=$newTagObject
    NewTagTarget=$newTagTarget
}
git status --short --branch
```

Expected: candidate worktree clean, tag target equals current HEAD, prior tag
identity recorded, release ZIP/hash recorded, and no remote push or release.

## Plan self-review

- Spec coverage: complete 15-dimensional INFO output, NOOP INFO, DEGRADED WARNING,
  stage/commit distinction, configuration behavior, privacy, never-raise,
  at-most-once, real AstrBot load, final package, and local retag all have explicit
  tasks.
- Placeholder scan: every implementation and acceptance step has concrete code,
  commands, and expected behavior.
- Type consistency: coordinator uses `diagnostic`; main validates the same nine
  fields; `dimensions_fxp6` and confidence remain integers; request marker stays
  `{status, code}`.
