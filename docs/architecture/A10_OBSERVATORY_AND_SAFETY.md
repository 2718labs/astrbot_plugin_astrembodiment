# A10 — Observatory、解释性与安全边界

## 目标

让开发者看见“她为什么这样行动”，但不让观测面成为第二个状态写入口，也不泄露原始对话。

## 只读投影

Observatory 可以显示：

- FormulaProfile / RuntimeEnvelope；
- active node/edge 数；
- 区域平均激活、E/I 比、疲劳、预测误差；
- 多尺度一致性 residual；
- relation residual 的归一化强度；
- 当前 action candidates 及 score 分解；
- claim/delivery/outcome 生命周期；
- Snapshot revision、Delta HWM、replay 状态；
- RSS、计算时延、缓存命中率；
- 最近 content-free Transition Receipts。

禁止显示：

- 原始文本；
- 能反推出原始内容的 free-form subtext；
- 语义摘要；
- 模型隐藏推理；
- 直接可编辑的神经元和 residual 表单。

## 不变量 residual

每次候选至少计算：

\[
\eta_{authority}=\|(I-P_\omega)\Delta z\|
\]

\[
\eta_{continuity}=\|S_{replay}-S_{candidate}\|
\]

\[
\eta_{energy}=\max(0,\Delta\mathcal H-y^Tu\Delta t)
\]

\[
\eta_{RG}=\|O_0(H^0)-O_L(H^L)\|
\]

\[
\eta_{capacity}=\max(0,|E|-E_{max})
\]

关键 residual 非零则拒绝提交。

## Expression Auditor

对最终 LLM 可见输出检查：

- 是否满足必须核验/认错/纠正；
- 是否超出 confidence ceiling；
- 是否在高风险错误中优先处理后果；
- 是否错误寻求用户安慰；
- 是否把礼貌纠错写成用户摩擦；
- 是否在不耐烦时出现辱骂、故意错误或必要信息缺失；
- 是否违反长度和 directness 合同。

失败时最多重写一次，仍失败则使用确定性安全模板。

## 管理动作

- `inspect`：只读；
- `verify_replay`：只读重放；
- `reset-affect`：高风险写操作，二次确认、原子执行、生成 admin receipt；
- `export-diagnostics`：只导出 content-free 数据；
- 禁止通过 WebUI 任意编辑单个 residual 或神经权重。

## MVP 验收

- Observatory API 零写入；
- 导出文件不包含原始消息；
- admin reset 有明确 scope、nonce 和 receipt；
- 失败重写不改变已提交行动；
- 资源不足时先丢诊断缓存，不丢权威状态。
