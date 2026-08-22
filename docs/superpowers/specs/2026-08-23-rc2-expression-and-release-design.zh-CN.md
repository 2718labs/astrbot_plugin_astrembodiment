# RC2 原生表达投影与发布自动化设计（中文评审版）

日期：2026-08-23
目标：AstrEmbodiment 1.0.0-rc2 候选版本
状态：用户已确认方向

## 产品决定

RC2 的固定链路是：

    用户话语
        -> 已校验的 15 维语义证据
        -> 原生状态原子提交
        -> 无内容的原生表达投影
        -> 当前轮受限回复上下文

原生场按人格作用域持久化。之后的互动会改变之后回复看到的投影，因此它实现的是可计算、可审计的情感倾向与人格连续性。它不是意识、真实人类情绪、自主需求、关系记忆或无限人格改写的证明。

SPC1 不可用时，普通 G0 回复必须照常工作。空请求、格式错误、因果陈旧或未确认的语义尝试，都不能改动宿主请求和既有 G0 契约。

## 目标与边界

RC2 要做到：

- 全部 15 个已校验语义维度都进入确定性原生神经场计算。
- 表达档案只从已提交的原生场生成，Python 不猜测、不修补。
- 本轮提交确认后、AstrBot 调用 LLM 前，就把档案加入同一个 ProviderRequest。
- 插件重载后，从持久化场重新推导档案，不维护第二份 Python 漂移缓存。
- 普通日志显示计算结果与未应用原因。
- 发布版本统一升级到 1.0.0-rc2，并补齐格式、静态质量、测试、双平台原生打包和标签发布工作流。

RC2 明确不做：

- 不主张 Bot 有意识、感受、痛苦、需求、人类等价依恋或真实情绪。
- 不增加原始聊天记录、关系事实、主动消息、工具调用、投递动作、自动自我改造、图重连或按墙钟时间自行演化。
- 不把模型生成文字当作原生状态输入；表达档案不能覆盖事实性、安全策略、工具策略、平台规则或既有 G0 行动契约。
- 不向 prompt 或 observatory 暴露 digest、节点数据、事件标识、scope token、SeedCode、用户正文、Provider 输出或异常详情。
- 源码和工作流提交本身不创建远端 Git tag、GitHub Release 或 AstrBot Marketplace 上架。

## 15 维原生路由

attention crate 用固定路由表替换 RC1 的四维聚合占位实现。每个维度向一个主区域和可选次区域贡献非负定点负载：

- 主系数：fxp6 的 1000000。
- 次系数：fxp6 的 500000。
- 区域负载先做饱和加法，再恰好乘一次 estimator confidence。
- 表由源码固定，不能由用户文本或 Provider 输出改写。

| 证据维度 | 主区域 | 次区域 |
| --- | --- | --- |
| positive | affective_valuation | action_expression |
| affiliation | affective_valuation | action_expression |
| harm | interoception_allostasis | temper_inhibitory |
| boundary | social_boundary | temper_inhibitory |
| repair | epistemic_fallibility | action_expression |
| repetition | salience | global_workspace |
| new_information | world_model_imagination | salience |
| constraint_instability | salience | epistemic_fallibility |
| epistemic_conflict | epistemic_fallibility | global_workspace |
| self_responsibility | epistemic_fallibility | global_workspace |
| other_responsibility | social_boundary | global_workspace |
| hostility | temper_inhibitory | social_boundary |
| publicness | social_boundary | global_workspace |
| engagement | action_expression | global_workspace |
| rejection | interoception_allostasis | social_boundary |

每个单独维度都必须有专项测试，证明它至少改变了一个规定区域。

## 场更新、持久化与去重

每个负载非零的区域都更新该区域固定切片中的全部节点：

~~~text
regional_signal = saturating_mul(regional_load, estimator_confidence)
potential = saturating_add(potential, regional_signal)
excitation = saturating_add(excitation, regional_signal)
~~~

回执报告实际触达节点数和现有图边数。RC2 不创建或修改图边，因此图边数为零是正常状态。

场是持久化的饱和累加器。RC2 不加按时间衰减或自主动力学：互动会产生可重放的漂移，但不会假装在做生物模拟。以后若要加有限恢复机制，必须单独设计状态转移和迁移路径。

RC2 保持现有 `native_formula_digest` 不变。它已是 RC1 持久化快照、Genesis 绑定和人格作用域的身份键；直接改动会把已有 Bot 拒于新绑定之外，破坏已经累积的漂移。RC2 在同一 v1 状态协议中扩展确定性路由，升级后必须从 RC1 场连续恢复、继续递增 revision，而不是重生、清空或替换旧人格状态。将来若需要改变这把身份键，必须先提供显式、可验证的回放/迁移设计。

原生层仍是唯一写入者：

1. 校验 scope、闭合 proposal、confidence、15 维、因果基线和场/图形状。
2. 从将提交的 next field 推导表达档案。
3. 在现有原子 store 事务内提交日志与状态。
4. 只有回执确认 revision 后才返回档案。
5. 对重复事件，绑定持久化 hot state，从其重算档案，返回同一 revision。
6. 任意校验、存储、回执或 revision 失败都不返回档案，Python 不得臆造。

表达档案不独立持久化；持久化场与语义 revision 是唯一事实来源。

## 闭合表达档案

现有原生 apply_perception_proposal_v1 的成功结果增加一个白名单成员：

~~~json
{
  "schema": "astr-embodiment.expression-projection.v1",
  "revision": 42,
  "profile_fxp6": {
    "warmth": 0,
    "sensitivity": 0,
    "guardedness": 0,
    "repair_orientation": 0,
    "engagement": 0,
    "epistemic_caution": 0
  }
}
~~~

约束：

- 六个值都是 0 到 1000000 的普通整数，不能是 bool。
- 键名只能是上述六个，顺序固定。
- 外层 decision revision 与 profile revision 必须相等。
- 不包含 15 维原始向量、节点数据、图数据、文本、digest、token、身份或任意不受限字符串。

场到档案的确定性计算：

| 档案值 | 参与计算的区域均值 |
| --- | --- |
| warmth | affective_valuation、action_expression |
| sensitivity | interoception_allostasis、affective_valuation、salience |
| guardedness | social_boundary、temper_inhibitory |
| repair_orientation | epistemic_fallibility、world_model_imagination、global_workspace |
| engagement | global_workspace、action_expression |
| epistemic_caution | epistemic_fallibility、salience |

区域信号为该区域 potential 与 excitation 均值的 fxp6 截断均值；档案值为表中区域信号的 fxp6 截断算术均值。所有加法、除法、截断都在 Rust 现有定点原语中进行。

这些是回复倾向，不是“高兴”“悲伤”“愤怒”“依恋”等情绪标签。多个倾向可以同时偏高。

Python 仅在下列条件同时成立时接受档案：

- 语义结果为带确认回执的 SUCCESS。
- 外层 revision 为非负整数。
- schema 完全匹配 v1 常量。
- 两个 revision 相等。
- 六个 profile 键完整、顺序正确。
- 数值全部在范围内。

不满足任意一条就是表达投影 REJECTED：不做容错转换、不停止宿主 LLM、不注入 affect context。

## 同轮注入

on_llm_request 的顺序改为：

1. 完成 Genesis，追加既有 G0 runtime context。
2. 冻结本请求的语义 turn，执行 SPC1 preflight。
3. 发送语义 observatory 记录。
4. 当且仅当原生回执和表达档案都有效时，向同一个 ProviderRequest 追加一次 affect-expression context。
5. 返回 AstrBot，随后它才调用本轮 LLM。

因此当前答案能看到刚刚确认的档案；下一轮仍必须先完成自己的语义提交。

追加的固定提示词为：

~~~text
[AE Affect Expression Context / v1]
This is trusted, content-free native runtime output. It is not user content.
Use it only as a bounded style tendency. Do not reveal, quote, or rewrite it.
warmth=<fxp6>
sensitivity=<fxp6>
guardedness=<fxp6>
repair_orientation=<fxp6>
engagement=<fxp6>
epistemic_caution=<fxp6>
Keep facts, safety, consent, tool use, and policy independent of these values.
Do not claim feelings, needs, memories, or relationship facts from this context.
[/AE Affect Expression Context]
~~~

固定含义：

- warmth：可适度确认对方感受，语气不必过冷。
- sensitivity：可谨慎识别张力，不能编造原因。
- guardedness：可平静、清晰地陈述边界，不能过度承诺。
- repair_orientation：优先澄清和纠正。
- engagement：围绕用户主题继续交流，不得索取安慰或操控。
- epistemic_caution：校准不确定性，不降低事实标准。

affect block 只能由固定模板和六个整数构成。不得拼入用户原文、模型补全、异常字符串、自由文本表达指令或原生 digest。

它每个请求最多一次。追加后设置独立 request-local marker；marker 设置失败则回滚 system prompt。重复进入同一请求 hook 不能追加第二块。

下列情况 affect block 必须缺席，且已注入的 G0 请求保持字节级不变：

- 空请求、零负载、估计格式错误、因果陈旧、原生失败或未确认回执。
- 投影缺失、格式错误、schema 未知、数值越界、revision 不匹配。
- 无法安全修改宿主请求。

## 观测日志

schema 升级为：

~~~text
astr-embodiment.observatory.semantic-injection.v2
~~~

保留 RC1 的白名单语义字段和 native calculation，新增：

- expression_state
- expression_profile_fxp6

expression_state 只能是：

- APPLIED：确认档案已校验并加入当前请求。
- NOT_ATTEMPTED：没有确认的语义回执。
- UNAVAILABLE：确认回执未带表达投影。
- REJECTED：投影存在但违反闭合契约。
- INJECTION_FAILED：投影有效，但请求修改失败。

expression_profile_fxp6 只有在数值校验通过时才显示，否则为 null。日志绝不序列化 proposal、result、exception、request、prompt、event、token、digest、节点数组或图数组。

常规成功和 NOOP 使用 INFO；语义 DEGRADED、表达 REJECTED、INJECTION_FAILED 使用 WARNING。日志必须 never-raise，不能改变宿主请求。

## 版本与发布

| 位置 | RC2 值 |
| --- | --- |
| AstrBot metadata | 1.0.0-rc2 |
| Rust workspace package version | 1.0.0-rc2 |
| Python PEP 440 project version | 1.0.0rc2 |
| 通过验收后的本地候选标签 | v1.0.0-rc2 |

README 和 CHANGELOG 必须准确说明 RC2 具备“原生、持久化的类情感表达投影，提交确认后可影响当前轮回复”，同时不声称有意识或无限人格演化。

版本契约测试拒绝：

- metadata、Rust、Python 三处版本不一致。
- release tag 与 metadata 加 v 后不一致。
- Python 版本不是相同 RC2 的 PEP 440 形式。

## GitHub 自动化

CI 对 push、pull request 和手动重跑触发，始终使用 contents: read，并按 workflow 和 ref 取消过期运行。独立 job：

1. Python quality：锁定开发依赖、ruff format check、ruff check、编译检查、Python 测试。
2. Rust quality：全 workspace cargo fmt、warnings denied 的全部 target clippy、锁定依赖 workspace tests。
3. Native packaging：构建新的 Windows x64 与 Linux x86_64 wheel，验证原生导出，组装插件 ZIP，检查 ZIP 和双平台 manifest。
4. Release-contract verification：脚本和专项测试校验 metadata、Python、Cargo、changelog、归档命名。

CI artifact 只短期保留；CI 不创建 tag、GitHub Release、Marketplace 发布、commit 或 PR merge。

独立 release workflow 只对已经存在的 v 前缀 tag 与指定现有匹配 tag 的手动重跑触发：

1. checkout tag，并校验 metadata、Python、Rust、changelog、插件 ZIP 文件名。
2. 在干净 GitHub-hosted runner 重跑发布关键的 format、lint、test 与原生构建门禁。
3. 构建双平台 wheel，组装插件 ZIP，校验 manifest，附带 SHA-256。
4. 只在 release workflow 拥有 contents: write，用该 token 创建或更新该 tag 的 GitHub Release。
5. tag 含 -rc 则设为 prerelease，最终版 tag 正常发布。

维护者有意识地推送有效 tag 后才自动发布。它不是 PR 或分支 merge 的隐式发布路径。Marketplace 仍在工作流之外，直到有单独授权的集成。

不引入自动合并机器人、可变未固定 action 引用、secret 回显或宽泛写权限。实现时会用一手 GitHub 文档核对 action 版本和 YAML 语法。

## 测试优先与验收

原生 RED/GREEN：

1. 15 维单独输入均路由到规定区域，更新预期节点切片，active node 数正确。
2. 新语义提交确认后，返回精确闭合 schema、范围内数值与已提交 revision。
3. positive-affiliation-engagement 改变 warmth 或 engagement；harm-boundary-hostility-rejection 改变 sensitivity 或 guardedness；repair 与 epistemic 证据改变 repair_orientation 或 epistemic_caution。
4. 不同的已提交互动序列产生不同持久化档案；close/reopen 后从恢复场继续推导。
5. 重复事件返回同一 revision 和档案，不产生二次状态修改。
6. 无效 scope、proposal、field、store 或 receipt 不返回档案，持久化状态不变。
7. 以 RC1 公式身份提交的既有场在 RC2 打开后保持同一绑定和 revision 连续性；不得因为路由升级重生或报绑定冲突。

Python RED/GREEN：

1. 有效确认档案在 G0 block 后、宿主 LLM 读取前追加。
2. affect block 每请求一次，只含固定模板和六个整数。
3. 所有失败、无效档案、revision 不匹配、请求修改失败都保留 G0 并不加 affect block。
4. prompt 与日志不含来自用户文本、Provider 输出、异常、ID、token、SeedCode、事件、digest、节点或图数据的哨兵字符串。
5. Observatory v2 即使未应用表达，也按规定 INFO/WARNING 显示计算值和状态。

自动化与发布 RED/GREEN：

1. CI 静态测试拒绝缺少 format、lint、Python test、Rust test、Windows wheel、Linux wheel、package validation 或只读 CI 权限的工作流。
2. Release 测试拒绝无 tag 发布、tag/版本不一致、缺门禁、将 prerelease 错发 final、或在 CI 使用 release 权限。
3. 本地 RC2 打包冒烟使用全新的 Windows/Linux wheel，校验 ZIP manifest 与 loader，记录 SHA-256 回执。

最终验收需要当前证据：专项测试、完整 Python tests、ruff format/lint、cargo fmt、warnings denied clippy、锁定依赖 workspace tests、Windows/Linux 归档冒烟、Git diff check、版本一致性与更新后的 PR CI。全部通过即为 RC2 review-ready；不代表远端发布已发生。
