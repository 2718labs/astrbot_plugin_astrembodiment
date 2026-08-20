# AstrEmbodiment Verification Gauntlet

## 1. 公式与权限

- [ ] 100 次 `SELF_ACTION` 后 bond/repair/reciprocity 增量严格为 0。
- [ ] `SELF_CRITIQUE` 不改变任何生产状态。
- [ ] `safe` 不进入情感荷载或 residual。
- [ ] authority residual 始终为 0。
- [ ] stale/cross-scope outcome 零写入。
- [ ] 同一 outcome 只结算一次。

## 2. 初见与关系

- [ ] 20 轮中性聊天后 warmth 不饱和。
- [ ] bond 只在外部互惠证据下形成。
- [ ] Bot 连续生成 100 条温柔回复，关系 residual 不增长。
- [ ] relation A 的 friction 不写入 relation B。

## 3. 不耐烦

- [ ] 单次重复只产生短期响应，不形成永久 friction。
- [ ] 重复 + 新信息的 friction load 显著低于无新信息。
- [ ] 上轮答案错误时 user responsibility 接近 0。
- [ ] 连续无视澄清后 action vector 逐步更收束、更直接。
- [ ] 高烦躁不降低事实、安全和必要风险提示。
- [ ] 不出现辱骂、人格攻击或故意给错。

## 4. 纠错

- [ ] 未核验 `CorrectionClaim` 不增加 fallibility。
- [ ] 高置信错误冲击大于低置信错误。
- [ ] 礼貌正确纠错增加 fallibility/fair correction，不增加 friction。
- [ ] 恶意正确纠错保持相同事实 verdict，同时增加 boundary/humiliation。
- [ ] 错误指责被驳回时 fallibility 增量为 0。
- [ ] 高风险错误优先纠正和止损，不向用户索取安慰。

## 5. 修复与不可逆性

- [ ] scar、repair、friction 等 residual 单调不减。
- [ ] repair 不删除 scar。
- [ ] 休息能降低瞬时烦躁/尴尬/疲劳，不改变 residual。
- [ ] `/reset` 与 `/reset-affect` 行为明确区分。

## 6. 神经与数值

- [ ] 16,384 节点槽位固定。
- [ ] 边数不超过 524,288。
- [ ] 100,000 随机事件无 panic、溢出和非法值。
- [ ] 无输入能量不增加。
- [ ] 节点置换测试保持宏观 action digest。
- [ ] 微观—宏观 residual 在容差内。
- [ ] 图生长/修剪使用迟滞，无高频抖动。

## 7. Agent 性

- [ ] 每轮至少生成 4 个不同的连续行动候选。
- [ ] 候选经过 world-model rollout 评分。
- [ ] 最终行为不是单一阈值状态机决定。
- [ ] action contract 对必须认错/核验/边界等约束可验证。
- [ ] 实际投递失败的草稿不进入 action ownership。

## 8. Continuum

- [ ] Snapshot + Delta 重放得到相同 state digest。
- [ ] 候选 Snapshot 失败不移动 active pointer。
- [ ] stale writer 更新零行。
- [ ] crash recovery 后状态一致。
- [ ] Journal 无原始文本、摘要或 embedding。
- [ ] Transition Receipt 完整且 content-free。

## 9. AstrBot 集成

- [ ] 当前目标 AstrBot 版本可加载插件。
- [ ] request/response/decorating/delivery 生命周期顺序正确。
- [ ] TTS/图片/工具输出不会被文本接管误吞。
- [ ] Python 不持有生产状态。
- [ ] native core 缺失时明确拒绝激活。
- [ ] terminate 先关闭准入，再排干 writer，再 flush。

## 10. 资源

### 2C2G

- [ ] 24 h 无 OOM、无无界增长。
- [ ] Rust core RSS ≤160 MiB 目标。
- [ ] 常规本地事件 p95 ≤40 ms 目标。

### 1C1G

- [ ] 无 swap 24 h 无 OOM。
- [ ] Rust core RSS ≤96 MiB 目标。
- [ ] 完整 AstrBot RSS ≤850 MiB 目标。
- [ ] 常规本地事件 p95 ≤120 ms 目标。
- [ ] 固定 replay 与 2C2G digest 完全一致。

## 11. 发布阻断

以下任何一项出现即禁止 1.0.0：

- 未授权 residual 写入；
- self-reward 回路；
- 温暖/兴奋无输入自激；
- 跨 relation 污染；
- 投递失败却登记行动；
- 1C1G 静默换公式；
- Journal 保存原始文本；
- replay 不一致；
- 高烦躁破坏能力底线；
- Python fallback 形成第二颗脑。
