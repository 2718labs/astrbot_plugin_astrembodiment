# Phase 0-B：即时本地估计与异步外部 teacher 补偿规格

## 1. 状态、硬边界、所有权

状态：`APPROVED_FOR_IMPLEMENTATION / NEW_INTEGRATION / DOCUMENT_ONLY`。依赖 Phase 0-A。持久化 map SHA-256 `2550039BFC27BB19A7ADEA2B40A1AAE4B83B45954F7462F3BFBB757C1E8ADD6C`。当前 HEAD 没有 worker/job persistence，既有 correction 类型 runtime/PyO3 明确 unsupported；这是新接入。

teacher 只复核冻结的当前用户输入 15D evidence。禁止结果修改 prompt/system prompt、temperature/top_p、tools、模型权重/routing、`expression_profile`、最终回复或历史 receipt/state；禁止读取 assistant reply、tools、完整历史、native 节点/边/ActionContract。它只能追加 native compensation checkpoint；expression 仅在后续正常 local perception 读取 committed state 后产生。

Agent B 独占新建 `local_semantic_estimator.py`、`teacher_compensation.py`、`teacher_worker.py`；修改 `semantic_estimator.py`、`coordinator.py`、`bridge.py`、`main.py`、`_conf_schema.json`；一个 focused Python test。B 不改 `crates/**` 或 SQLite。

## 2. 时序/local estimator

~~~text
on_llm_request -> freeze text once -> synchronous local estimate (no await/I/O)
 -> native local perception -> current normal expression
 -> native enqueue text-free metadata -> worker.notify(ephemeral raw text)
 -> current conversation continues without waiting

worker -> claim -> dedicated teacher(current text only) -> strict 15D parse
 -> current cursor + verified telemetry -> candidate B -> native compensation
 -> terminal receipt; never touches ProviderRequest/response/expression callback
~~~

local estimator：Unicode NFKC/casefold，4096 codepoint 上限，冻结无回溯 literal lexicon。每维 longest-literal 去重：`intensity=min(S,max_weight+50000*additional_matches)`；PRESENT confidence `min(850000,600000+50000*matches)`；未命中 ABSENT/0/400000。规则 digest 进入 formula；aggregate confidence 取 min，逐维 confidence 放 job descriptor。

teacher：专用 provider、固定 prompt/schema、`contexts=None/tools=None/temperature=0`，prompt 仅当前用户文本，输出严格 V3 15 槽。

## 3. Job/隐私/并发

native 只持久化 job/scope/source-event/source-text digest、local 15D/confidence、provider/model/prompt/schema/formula digest、created/expiry。raw text 仅同进程有界 queue。重启无 raw text 的 pending/claimed job -> `ABANDONED_INPUT_UNAVAILABLE`，不得猜测/重放。

`job_id=SHA256(domain||scope||source_event||provider/prompt/schema digests)`；native unique/lease。compensation 以完成时当前 semantic revision 为 base；source event 只作因果来源。第一次 stale 重读重算一次，第二次终止。同 job返回原 receipt。

## 4. 候选 B

每维参数：`TEACHER_CONF_MIN=900000`、`ENTER=200000`、`EXIT=100000`、`U_MAX=250000`、`RISE_MAX=50000`、`FALL_MAX=100000`。无跨维 sum/共享预算/借额。生产 policy/consistency artifact 缺失即 disabled。

~~~text
err[d]=teacher[d]-local[d]
eligible[d]=teacher_conf[d]>=900000 AND abs(err[d])>=200000
n=min(energy_headroom,capacity_headroom,residual_health)
q[d]=mul6(teacher_conf[d],S-floor(local_conf[d]/2))
gain[d]=mul6(mul6(q[d],consistency[d]),n)
target[d]=clamp(smul6(err[d],gain[d]),-U_MAX[d],U_MAX[d])

unavailable/conf too low -> hold u_prev; NO_STATE_ADVANCE
u_prev==0 and abs(err)<ENTER -> desired=0
u_prev!=0 and abs(err)<=EXIT -> desired=0
abs(err)>=ENTER -> desired=target
otherwise -> desired=u_prev
sign reversal -> desired=0 first
step_cap=mul6(RISE_MAX,n) if abs(desired)>abs(u_prev) else FALL_MAX
u_next=clamp(u_prev+clamp(desired-u_prev,-step_cap,step_cap),-U_MAX,U_MAX)
~~~

至少一维 eligible 才提交；按绝对值选择 rise/fall，使正负方向的加大都受 bottleneck 限制。

## 5. Receipt/lifecycle/验收

receipt 含 job/event/source event、base/next、policy/teacher/telemetry/checkpoint digests、eligible/changed counts、15D u_next、status、receipt digest；`expression_projection=null`。状态 `PENDING->CLAIMED->COMMITTED|NO_CHANGE|REJECTED|EXPIRED`；restart 无 raw text -> abandoned。

worker initialize 后 start；terminate 停止接单、取消 provider await、封存 job，再 native flush。日志无 raw text/15D/completion。

最低验收：G: pycache compileall exit 0；never-returning teacher 不延迟 current closure；仅逐维 `conf>=900000 && abs(err)>=200000` 改该维；当前 request 的 prompt/参数/reply/expression 不因 teacher 改变；compensation append-only/幂等/无 raw text，只有后续 normal perception 观察它。
