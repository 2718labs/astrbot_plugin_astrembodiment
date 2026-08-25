# Phase 0-B Async Teacher Compensation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** local estimator 永远即时提交；teacher 只异步追加 native compensation checkpoint。

**Architecture:** Python 负责 local/teacher/worker/JSON；Rust 是 job/telemetry/formula/CAS/receipt 权威。worker 不接受 ProviderRequest/response/expression callback。

**Tech Stack:** Python 3.12+, asyncio, strict JSON, NativeBridge, Phase 0-A ABI.

---

只修改规格 B 文件，不改 `crates/**`。Phase 0-A ABI 缺失则 `BLOCKED_NATIVE_ABI`，禁止 fallback SQLite/receipt。

### Task 1: Immediate local estimator

**Files:** Create `astr_embodiment/local_semantic_estimator.py`; Modify `semantic_estimator.py`, `coordinator.py`.

- [ ] 同步 `estimate_local_v1` + frozen lexicon/digest，无 await/provider/file/network。
- [ ] 共享 V3 parser；外部 provider 入口显式 teacher。
- [ ] current turn 先 local proposal→native，不等 teacher。
- [ ] compileall；Commit `feat(host): add immediate local semantic estimator`。

### Task 2: Teacher/candidate B

**Files:** Create `astr_embodiment/teacher_compensation.py`; Modify `semantic_estimator.py`.

- [ ] closed job/policy/result；teacher 只接 current text 和 15D schema。
- [ ] FxP6 mirror、`conf>=900000`、`abs(err)>=200000`、迟滞、按绝对值 rise/fall、逐维无共享预算。
- [ ] builder 只产生 text-free proposal，无 prompt/expression/reply 字段；native 独立重算。
- [ ] compileall；Commit `feat(host): define teacher compensation contract`。

### Task 3: Bridge seam

**Files:** Modify `astr_embodiment/bridge.py`.

- [ ] 验证 closure v2/telemetry keys、PREPARE、digest/revision/limits、`native_gate=min(...)`。
- [ ] legacy 只 `UNAVAILABLE_LEGACY`，旧零 residual 不当健康。
- [ ] enqueue/claim/abandon/reject/apply compensation；symbol/schema 错误 closed degraded。
- [ ] receipt `expression_projection is None` 且 dedup digest 一致。
- [ ] compileall；Commit `feat(host): validate phase0 native teacher abi`。

### Task 4: Worker

**Files:** Create `astr_embodiment/teacher_worker.py`; Modify `coordinator.py`.

- [ ] bounded queue+pending set+lock 的 start/notify/close；raw text 仅 queue item。
- [ ] native text-free enqueue 后入队；同 job coalesce。
- [ ] claim→teacher→parse→current telemetry→compensation；stale 最多重算一次。
- [ ] timeout/malformed/shutdown terminalize，不影响 local；日志无 raw text/15D/completion。
- [ ] compileall；Commit `feat(host): run asynchronous teacher worker`。

### Task 5: Lifecycle/硬隔离

**Files:** Modify `main.py`, `_conf_schema.json`.

- [ ] 仅 enabled/provider/timeout/queue/max-age/policy；默认 disabled，缺 policy fail closed。
- [ ] initialize start；terminate close worker 后 native flush。
- [ ] on_llm_request normal closure 后只 notify，不 await/传 request。
- [ ] teacher completion 禁止调用 request/expression/response/send APIs。
- [ ] compileall；Commit `feat(host): wire teacher lifecycle without response authority`。

### Task 6: 两个 focused probes/编译

**Files:** Create `tests/test_teacher_compensation.py`.

- [ ] never-returning teacher 不延迟 current closure；prompt/system/temperature/top_p/expression/reply 不变。
- [ ] threshold/B、无跨维预算、one stale retry、job 幂等、raw text 不进 durable payload。
- [ ] 仅运行 `python -m pytest -q tests/test_teacher_compensation.py`。
- [ ] G: 定向 pycache：

~~~powershell
$env:PYTHONPYCACHEPREFIX='G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\phase0-build-20260825\pycache'
python -m compileall -q -f -o 0 -o 1 main.py astr_embodiment python
~~~

- [ ] exit 0；`git diff --name-only` 仅 B 边界；Commit `test(host): verify isolated teacher compensation`。不声称 release acceptance。
