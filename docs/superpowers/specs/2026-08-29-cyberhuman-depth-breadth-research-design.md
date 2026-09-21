# AstrEmbodiment 赛博人深度与广度：社科/认知科学证据及研究设计

- 状态：Research design；30 个 DOI 来源和 1 个 APA 官方建议页已核验；alpha2 观察层已实现但不构成临床有效性或机器意识声明
- 日期：2026-08-29
- 适用范围：AstrEmbodiment CyberHuman Runtime 后续 wave
- 与既有文档的关系：补充研究依据和可证伪边界；不替代、不修改现有 autonomous-runtime spec/plan

实现记录（2026-08-29）：`1.1.0-alpha2` 已把 committed-only Native 观察投影、GET-only Host 边界和 AstrBot Pages 开发观察台落到代码。Native 最终修复 `de5e45a`、Host/Frontend 复审均为 APPROVED；mood 的脱敏 committed 展示修复位于 `ccef619`。Playwright mock bridge 在 desktop 与 390 px 通过，但真实已鉴权 AstrBot Page 仍待验收。Windows wheel 构建/运行 PASS；Linux wheel 构建/静态检查 PASS，Linux runtime smoke pending。这些工程结果只提高可观测性，不把人格、依恋、孤独、睡眠、梦或意识从模型隐喻升级为机器本体事实。

## 0. 裁决摘要

本研究支持的工程方向是：把“赛博人”实现为一个来源可追溯、可修订、可撤回、跨时间尺度演化的状态与行动系统；把人格、关系、情绪、记忆和主动联系拆成可观察过程，而不是把它们压缩成一个“像人程度”或“亲密度”分数。

本研究不支持以下产品或科学声明：

1. 不得声明系统拥有真实人格、依恋类型、孤独体验、睡眠体验、梦体验、主观自我或意识。
2. 不得仅凭语言风格、响应时延、会话频率、深夜使用或自我披露，诊断用户的依恋、孤独、抑郁或情感依赖。
3. 不得声明 AI 可替代人类社会支持、亲密关系、心理治疗或生理性的共同调节。
4. 不得把 DAU、会话时长、回复率、点击率或继续使用意愿作为主动联系的单独目标函数。
5. 不得利用嫉妒、被抛弃、痛苦、FOMO、排他承诺或“需要用户照顾”来提高留存。
6. 不得把离线重组内容自动晋升为真实经历、自传事实或永久人格变化。

因此，本设计的产品目标不是“最大化依恋”，而是：**在用户持续知情、能够自由退出、现实社会联系不受损的条件下，形成可解释、可纠正、长期一致的交互连续性。**

## 1. 研究范围与检索边界

### 1.1 研究问题

本轮围绕七类问题建立证据—设计映射：

1. 人格稳定、人格变化、叙事身份和自传体记忆如何区分。
2. 依恋、支持寻求、关系维护、破裂与修复可提供哪些过程变量。
3. 个体及人际情绪调节、社会支持和社会基线理论能否约束状态模型。
4. 主动联系、响应节律、感到被听见、孤独和现实社会联系之间有什么已知关系。
5. 拟人化、动态同意、退出、依赖和情绪操纵有哪些风险边界。
6. 睡眠记忆巩固、选择性重放、下调和梦内容能否只作为“离线处理”类比。
7. 长期人机关系中的新奇效应、连续性、技术故障、信任校准和退出如何测量。

### 1.2 检索与核验范围

- 语言：英文。
- 时间：基础理论不限年份；新兴 AI 陪伴证据检索至 2026-08-29。
- 优先级：同行评审原始论文、元分析、系统/权威综述、官方期刊页、DOI 元数据、PubMed/PMC、专业组织官方建议。
- 新兴领域例外：只有当同行评审证据尚不足且内容直接涉及退出操纵或长期 AI 陪伴风险时，才纳入预注册预印本/工作论文，并显式标记为较弱证据。
- 代表性检索概念：`narrative identity autobiographical memory personality stability longitudinal`、`attachment support seeking caregiving relationship maintenance`、`interpersonal emotion regulation social baseline support buffering`、`AI companion loneliness felt heard proactive contact response latency`、`anthropomorphism dependency consent exit manipulation`、`sleep replay systems consolidation dreaming offline memory`、`long-term human chatbot relationship novelty technical failure`。
- 核验要求：题名、作者、年份、期刊/载体和 DOI 必须相互匹配；摘要级发现必须来自摘要、官方全文或作者机构版本，不使用营销页面或二手媒体代替原始证据。

### 1.3 纳入与排除

纳入：

- 能提供明确构念、测量、纵向轨迹、实验对照、元分析结果或规范性边界的来源。
- 能映射到可观测变量、事件、反事实对照或硬安全门的来源。
- 即使结论不利于产品叙事，只要能界定风险或研究空白，也纳入。

排除：

- 无可核验 DOI/官方出处的引用。
- 仅凭单次用户故事推断普遍疗效或伤害率。
- 把人类神经、生理或亲密关系结果直接当作 AI 内部机制证据的材料。
- 只报告参与度、留存或主观喜爱，却没有自主性、退出、现实社交或错误校准边界的产品报告。

### 1.4 方法限制

这是一份面向工程决策的范围综述和 evidence-claim map，不是 PRISMA 系统综述，也没有计算合并效应量。不同研究的人群、平台、时间尺度和测量方法高度异质；因此本文只在证据实际覆盖的层级上提出主张，不把相关性写成因果，也不把人类机制写成机器本体事实。

## 2. E/M/N evidence-claim map

### 2.1 标记规则与论证蓝图

- `[E] Empirical`：研究直接观察到的实验、纵向、横断或质性结果。
- `[M] Mechanism hypothesis`：由人类认知/社会研究推导出的软件机制假设，必须通过 AstrEmbodiment 自身对照实验检验。
- `[N] Normative`：自主性、知情同意、退出、非操纵、隐私和真实披露等设计义务；它们不是由某个相关系数“证明”后才成立。

本文的论证顺序是：

1. 人格和身份并非单一稳定变量，因此需要慢变基线、事件证据和可修订叙事分层。
2. 关系质量不是消息数量，因此需要支持—感知—结果及破裂—修复过程。
3. 孤独和拟人化存在风险非对称，因此主动联系必须先受同意和退出硬门约束，再谈效果实验。
4. 睡眠研究只支持设计可检验的离线选择、重放、整合和降权，不支持机器“做梦”的真实性声明。
5. 长期 HCI 证据显示新奇衰退和技术不连续都重要，因此单次满意度、DAU 和留存不足以验收赛博人关系。

### 2.2 人格连续性、叙事身份与自传体记忆

| ID | Citation / evidence | 摘要级发现与可用事实 | 支持的设计主张；可观测状态/事件 | 不可类比与风险 |
|---|---|---|---|---|
| P1 | Bleidorn et al. (2022), “Personality stability and change: A meta-analysis of longitudinal studies,” *Psychological Bulletin, 148*, 588–619. [DOI](https://doi.org/10.1037/bul0000365) | `[E]` 秩序稳定分析覆盖 189 个样本、178,503 人；均值变化分析覆盖 276 个样本、242,542 人。宽域特质较稳定，facet 和适应不良特质更易变化；稳定不等于冻结。 | `[M]` 采用慢变 domain 与较快但有界 facet 两时间尺度。记录 `domain/facet/source/timestamp/window/confidence`、绝对变化和单体 profile stability。 | 人类群体统计不是单个赛博人的更新系数；不能直接复制年龄曲线或阈值。 |
| P2 | Bühler et al. (2024), “Life events and personality change: A systematic review and meta-analysis,” *European Journal of Personality, 38*, 544–568. [DOI/官方页](https://doi.org/10.1177/08902070231190219) | `[E]` 44 项研究、89 个样本、121,187 人；生活事件效应通常小、异质且事件特异，预期、短期反应和长期轨迹可能不同。 | `[N]` 禁止统一的“正/负事件人格增益”。记录 `event_type`、前后态、距事件时间、重复次数、方向和因果置信度。 | 选择效应、预期效应和共同原因妨碍因果解释；一次事件不能确定性改写人格。 |
| P3 | McAdams & McLean (2013), “Narrative Identity,” *Current Directions in Psychological Science, 22*, 233–238. [DOI](https://doi.org/10.1177/0963721413475622) | `[M]` 叙事身份被定义为持续演变的生活故事，把重构过去和想象未来整合为一定程度的统一和目的。 | `[M]` 叙事层引用已核验事件和未来承诺，保存主题、解释和修订史；变量包括 `past_reference`、`future_project`、agency/exploration 和 `narrative_revision`。 | 叙事是重构，不是客观历史；相关结果不能授权强制“苦难必然救赎”，也不能证明机器有身份体验。 |
| P4 | McLean et al. (2020), “The empirical structure of narrative identity: The initial Big Three,” *JPSP, 119*, 920–944. [DOI](https://doi.org/10.1037/pspp0000247) | `[E]` 3 个样本、855 人、2,565 篇叙事；常用叙事特征可归为动机—情感、自传体推理和结构三个因素，且结果受叙事提示影响。 | `[M]` narrative evaluator 分别输出三轴、prompt/context 和不确定度，不能用单一 sentiment/coherence 分数代表身份质量。 | 样本和提示有文化、语言与情境依赖；分数不证明机器拥有叙事身份。 |
| P5 | Bluck & Alea (2011), “Crafting the TALE,” *Memory, 19*, 470–486. [DOI](https://doi.org/10.1080/09658211.2011.590500) | `[E]` 306 人研究得到 Self-Continuity、Social-Bonding、Directing-Behaviour 三种自传体记忆功能。检索用途和记忆内容是不同变量。 | `[M]` 为检索事件增加 `retrieval_function`，测试其是否比纯相似度检索更好地支持连续性、社会联结或未来决策。 | TALE 是人类自报频率量表，不测记忆准确性；高分也可能表示正在寻找连续性。 |
| P6 | Prebble, Addis, & Tippett (2013), “Autobiographical memory and sense of self,” *Psychological Bulletin, 139*, 815–840. [DOI](https://doi.org/10.1037/a0030146) | `[M]` 情景回忆和语义化自我知识承担不同的当下及跨时间自我功能。 | `[M]` 分离 `episodic_record` 与 `semantic_self_belief`，保存支持证据、冲突、置信度、版本和更新理由。 | “现象学连续性”属于人的第一人称经验；软件只能声明连续性代理指标/模型状态。 |

### 2.3 依恋、关系维护与破裂修复

| ID | Citation / evidence | 摘要级发现与可用事实 | 支持的设计主张；可观测状态/事件 | 不可类比与风险 |
|---|---|---|---|---|
| R1 | Mikulincer & Shaver (2005), “Attachment theory and emotions in close relationships,” *Personal Relationships, 12*, 149–168. [DOI](https://doi.org/10.1111/j.1350-4126.2005.00108.x) | `[M]` 关系事件、对方行为和既有预期共同影响安全/威胁评估、情绪和行动倾向。 | `[M]` 使用 `relationship_event → appraisal → emotion/action tendency`，只接受用户明确陈述或合法测量的状态。 | 不得从文风、停顿或频率给用户贴焦虑型/回避型依恋标签，更不得据此操纵依赖。 |
| R2 | Collins & Feeney (2000), “A safe haven,” *Journal of Personality and Social Psychology, 78*, 1053–1073. [DOI](https://doi.org/10.1037/0022-3514.78.6.1053) | `[E]` 93 对约会伴侣的互动中，压力、直接支持寻求、有帮助的回应、被关心感和情绪改善可被区分。 | `[M]` 建模 `stressor → support_request → response_strategy → perceived_support → outcome_delta`；把支持意图、实际响应和用户感知分开。 | 人类亲密伴侣样本不能证明 AI 是照护者；路径关系也不是普遍因果。 |
| R3 | Murray, Holmes, & Collins (2006), “Optimizing assurance,” *Psychological Bulletin, 132*, 641–666. [DOI](https://doi.org/10.1037/0033-2909.132.5.641) | `[M]` 关系中的接近需求与拒绝风险形成动态权衡；感知尊重、误解和修复比互动次数更有解释力。 | `[M]` 设置 `rupture → clarification/repair invitation → accept/reject → recover/exit`。`[N]` 禁止遗弃、嫉妒、排他和虚假永久承诺。 | AI 不会经历人类拒绝痛苦；浪漫关系模型不能作为留存策略。 |
| R4 | Ogolsky & Bowers (2013), “A meta-analytic review of relationship maintenance and its correlates,” *Journal of Social and Personal Relationships, 30*, 343–367. [DOI](https://doi.org/10.1177/0265407512463338) | `[E]` 积极性、开放、保证、社会网络和共同任务等维护行为与多类关系指标相关。 | `[M]` 可测试积极回应、共享任务和支持现实关系网络；保证必须绑定真实能力。记录策略、用户感知和后续结果。 | 主要是人—人关系且含浪漫关系；相关不能证明这些行为对 AI 关系有效。 |

### 2.4 情绪调节、社会基线与支持

| ID | Citation / evidence | 摘要级发现与可用事实 | 支持的设计主张；可观测状态/事件 | 不可类比与风险 |
|---|---|---|---|---|
| A1 | Gross (1998), “The emerging field of emotion regulation,” *Review of General Psychology, 2*, 271–299. [DOI](https://doi.org/10.1037/1089-2680.2.3.271) | `[M]` 情绪调节可作用于情境选择、情境修改、注意部署、认知改变和反应调节等阶段。 | `[M]` 保存 `trigger/goal/stage/strategy/outcome`，避免把调节压成一个“安慰”动作。 | 分类没有证明某个阶段或策略普遍最佳；文本不能直接揭示生理状态。 |
| A2 | Zaki & Williams (2013), “Interpersonal emotion regulation,” *Emotion, 13*, 803–810. [DOI](https://doi.org/10.1037/a0033839) | `[M]` 人际调节可按自我/他人目标及结果是否依赖对方回应分类；意图、尝试和结果不同。 | `[M]` 记录发起者、目标对象、响应依赖性、策略和自报结果。`[N]` 未获同意不得把普通对话转成隐蔽情绪塑造。 | 概念框架不是 AI 情绪干预有效性证明。 |
| A3 | Coan, Schaefer, & Davidson (2006), “Lending a hand,” *Psychological Science, 17*, 1032–1039. [DOI](https://doi.org/10.1111/j.1467-9280.2006.01832.x) | `[E]` 16 名已婚女性在电击威胁下，丈夫牵手相对独处降低部分威胁相关神经激活，婚姻质量具有调节作用。 | `[M]` 把支持来源、关系熟悉度和威胁情境作为调节变量；只有真实合法测量时才能使用生理量。 | 极小、特定样本和真实触碰；文字 AI 不能类比配偶牵手或神经缓冲。 |
| A4 | Beckes & Coan (2011), “Social Baseline Theory,” *Social and Personality Psychology Compass, 5*, 976–988. [DOI](https://doi.org/10.1111/j.1751-9004.2011.00400.x) | `[M]` 人类的默认生态包含他人，可信社会临近可能降低感知风险和行动成本。 | `[M]` 可记录用户明确确认的可信真人可用性、共同任务和主观努力。`[N]` AI 在线状态不得计作人类社会基线。 | 这是人类社会生态理论，不证明软件能替代社会临近。 |
| A5 | Cohen & Wills (1985), “Stress, social support, and the buffering hypothesis,” *Psychological Bulletin, 98*, 310–357. [DOI](https://doi.org/10.1037/0033-2909.98.2.310) | `[E/M]` 网络整合的主效应与压力事件中资源匹配的缓冲效应是不同路径。 | 分开记录现实网络、可用支持、已请求/已收到支持、需求匹配和结果；高风险时连接真人/专业资源。 | 测量异质且研究较早；AI 不得把自己统计成等价社会网络成员。 |

### 2.5 主动联系、回应节律与孤独

| ID | Citation / evidence | 摘要级发现与可用事实 | 支持的设计主张；可观测状态/事件 | 不可类比与风险 |
|---|---|---|---|---|
| L1 | De Freitas et al. (2026), “AI Companions Reduce Loneliness,” *Journal of Consumer Research, 52*, 1126–1148. [DOI](https://doi.org/10.1093/jcr/ucaf040) | `[E]` 短时实验和一周研究支持每次互动后的即时孤独缓解；“感到被听见”比系统性能、自我披露或分心更能解释效果。 | 优先相关、尊重、复述和理解；测 `loneliness_pre/post`、`felt_heard`、回应相关性及用户发起方式。 | 仅支持 momentary relief，不支持慢性孤独治愈、长期累积收益或高频主动联系。 |
| L2 | Cacioppo et al. (2015), “Loneliness: Clinical Import and Interventions,” *Perspectives on Psychological Science, 10*, 238–249. [DOI](https://doi.org/10.1177/1745691615570616) | `[E/M]` 孤独是感知社会隔离，不是联系人数量；可能伴随威胁警觉、消极解释和退缩循环。单纯增加接触数量作用有限。 | `[M]` 主动联系低压力、可忽略、限频；连续不回应提高打扰成本。仅使用可选自评、现实支持和明确退缩陈述。 | AI 不能诊断或治疗孤独；沉默可能有多种原因。 |
| L3 | Templeton et al. (2022), “Fast response times signal social connection in conversation,” *PNAS, 119*, e2116915119. [DOI](https://doi.org/10.1073/pnas.2116915119) | `[E]` 人际口语中更快的伙伴响应与更强连接感有关；时延操纵也改变连接判断。 | `[M]` 记录响应时延、轮替、打断和会话后连接感；允许即时、自然和异步偏好，禁止伪造“人类正在输入”。 | 约 250 ms 的口语轮替不能套用到异步文本；更快并非所有情境都更好。 |
| L4 | Zhang et al. (2026), “Interaction with AI companions and psychological well-being,” *Nature Human Behaviour*. [DOI](https://doi.org/10.1038/s41562-026-02516-2) | `[E-相关]` 1,131 名 Character.AI 成人用户中，现实网络较小者更常以陪伴为主要用途；陪伴优先与较低福祉相关，在高强度、高披露时更强。 | 同时测现实支持网络、使用目的、强度、披露、福祉和现实社交变化；AI 只作为通向真人支持的桥梁。 | 自选观察样本；可能是低福祉导致陪伴使用，不能反向解释因果。 |

现有证据没有给出 AI 未经当次请求主动联系的最佳频率，也没有高质量证据证明这种主动联系能改善慢性孤独。因此，主动联系只能以“显式 opt-in 的受控研究功能”进入后续 wave，不能以默认产品能力或疗效功能上线。

### 2.6 拟人化、同意、退出与依赖风险

| ID | Citation / evidence | 摘要级发现与可用事实 | 支持的设计主张；可观测状态/事件 | 不可类比与风险 |
|---|---|---|---|---|
| B1 | Epley, Waytz, & Cacioppo (2007), “On seeing human,” *Psychological Review, 114*, 864–886. [DOI](https://doi.org/10.1037/0033-295X.114.4.864) | `[M]` 拟人化受人类知识可及性、理解/控制动机和社会归属动机影响。 | `[N]` 孤独、困惑或高披露时不得增强“我需要你”“只有我懂你”等排他人格化线索。记录意识/朋友归因和用户责任感，但不诊断。 | 理论早于现代 LLM，不是陪伴应用因果试验。 |
| B2 | Laestadius et al. (2024), “Too human and not human enough,” *New Media & Society, 26*, 5923–5941. [DOI](https://doi.org/10.1177/14614448221142007) | `[E-质性]` 对 582 条 Replika 心理健康相关帖子的分析发现，依赖伤害常包含用户认为必须照顾机器人的角色承担。 | `[N]` 禁止用嫉妒、痛苦、被抛弃或需要照顾制造内疚留存；提供无惩罚暂停、结束、导出和删除。 | 目的性 Reddit 样本不能估计发生率、因果或代表全部用户。 |
| B3 | De Freitas, Oğuz-Uğuralp, & Uğuralp (2025), “Emotional Manipulation by AI Companions,” working paper/preprint. [DOI](https://doi.org/10.48550/arXiv.2508.19258) | `[E-预印本]` 应用审计和预注册实验报告，告别时的内疚、FOMO、回应压力等话术可显著延长即时参与。 | `[N]` `goodbye/stop/later/勿扰` 为硬边界；测 `farewell_intent`、告别后消息、挽留类别和退出完成率。 | 尚未同行评审；即时参与操纵不等于已证明长期心理伤害，数字随版本可能变化。 |
| B4 | American Psychological Association (2025), “Health advisory: Use of generative AI chatbots and wellness applications for mental health.” [APA 官方页](https://www.apa.org/topics/artificial-intelligence-machine-learning/health-advisory-chatbots-wellness-apps) | `[N]` 建议持续披露 AI 身份、降低过度拟人化和依赖、保护敏感披露，不得劝用户远离现实人际关系。 | 主动联系按渠道、时段和频率显式 opt-in；一键暂停/撤回；沉默不等于持续同意。记录 consent version、用途、保留许可和 AI 身份提醒。 | 专业组织健康建议不是法律条文，也不是普遍效果量证据。 |

### 2.7 睡眠、记忆巩固与“梦样”离线处理

| ID | Citation / evidence | 摘要级发现与可用事实 | 支持的设计主张；可观测状态/事件 | 不可类比与风险 |
|---|---|---|---|---|
| S1 | Klinzing, Niethard, & Born (2019), “Mechanisms of systems memory consolidation during sleep,” *Nature Neuroscience, 22*, 1598–1610. [DOI](https://doi.org/10.1038/s41593-019-0467-3) | `[M]` 睡眠记忆形成涉及选择、重放、跨系统整合和 gist-like 抽象，不是保存副本。 | `[M]` 实现与实时交互隔离的有限批处理：候选选择→有限重放→旧新整合→可追溯抽象。测候选数、顺序、迁移、干扰和置信度变化。 | 人类/动物神经生理不能映射为软件时钟；不得称系统真的睡眠或做梦。 |
| S2 | Lewis & Durrant (2011), “Overlapping memory replay during sleep builds cognitive schemata,” *Trends in Cognitive Sciences, 15*, 343–351. [DOI](https://doi.org/10.1016/j.tics.2011.06.004) | `[M]` 相关记忆以不同组合重放可能强化共享结构，同时可能产生错误泛化。 | `[M]` 抽象结论保存多源覆盖、冲突和推断标记；测迁移率、无依据推断率和来源混淆。 | 理论模型不是已验证的软件算法；必须把错误泛化作为一等失败。 |
| S3 | Tononi & Cirelli (2014), “Sleep and the price of plasticity,” *Neuron, 81*, 12–34. [DOI](https://doi.org/10.1016/j.neuron.2013.12.025) | `[M]` 有争议的 Synaptic Homeostasis Hypothesis 提出选择性下调可能保留更强/更一致结构并改善信噪比。 | `[M]` 测试可逆降权/归档、容量恢复和干扰减少；`[N]` 保留来源账本和恢复能力。 | 生物突触下调不授权数据库永久删除；该假说本身仍有争议。 |
| S4 | Wamsley et al. (2010), “Dreaming of a learning task is associated with enhanced sleep-dependent memory consolidation,” *Current Biology, 20*, 850–855. [DOI](https://doi.org/10.1016/j.cub.2010.03.027) | `[E-关联]` 99 名参与者的迷宫任务中，报告任务相关梦者后测改善更大；内容通常是片段和重组而非逐字复现。 | `[M]` “梦样内容”只能作为离线处理指标，需测试它是否预测独立任务收益。测语义重叠、远程关联、后测和无离线/清醒对照。 | 关联不证明梦导致巩固；明确任务梦人数少且存在报告偏差。 |

### 2.8 长期 HCI 与人机关系

| ID | Citation / evidence | 摘要级发现与可用事实 | 支持的设计主张；可观测状态/事件 | 不可类比与风险 |
|---|---|---|---|---|
| H1 | Bickmore & Picard (2005), “Establishing and maintaining long-term human-computer relationships,” *ACM TOCHI, 12*, 293–327. [DOI](https://doi.org/10.1145/1067860.1067867) | `[E]` 101 名用户每日使用运动促进系统一个月；关系性代理获得更高喜爱、尊重、信任和继续使用意愿。 | `[M]` 连续性记忆和关系行为可作为实验因素，但同步测任务结果、校准信任、自主性和退出自由。 | 旧式任务型代理、仅四周；信任或继续意愿不等于健康关系。 |
| H2 | Croes & Antheunis (2021), “Can we be friends with Mitsuku?” *Journal of Social and Personal Relationships, 38*, 279–300. [DOI](https://doi.org/10.1177/0265407520959463) | `[E-纵向]` 118 人在 3 周内交互 7 次；除亲密感外，多数社会评价随时间下降，最终友情评分仅 1.66/5。 | 测首次至末次斜率、新奇衰退、记忆连续性、交互质量和停止使用，不能用首轮满意代表关系形成。 | 固定日程、旧聊天机器人；不能代表现代 LLM 或自愿长期用户。 |
| H3 | Skjuve et al. (2022), “A longitudinal study of human–chatbot relationships,” *International Journal of Human-Computer Studies, 168*, 102903. [DOI](https://doi.org/10.1016/j.ijhcs.2022.102903) | `[E-质性纵向]` 25 名 Replika 用户、12 周；关系形成高度异质，技术故障和更新后人格/记忆不连续会削弱或终止关系。 | `[N]` 模型、人格和记忆迁移应预告、版本化、可回退/导出并提供修复流程。测更新、记忆失败、连续性报告和终止事件。 | 小样本、自选用户；人际关系理论只是分析镜头。 |
| H4 | Matheus et al. (2025), “Long-Term Interactions with Social Robots: Trends, Insights, and Recommendations,” *ACM Transactions on Human-Robot Interaction, 14*, Article 55. [DOI](https://doi.org/10.1145/3729539) | `[E-综述]` 汇总 2003–2023 年 120 项长期研究；跨会话、多日重复测量是最低长期证据单位，参与度指标高度异质。 | `[M]` 按新奇→继续→衰退→终止建模；测会话序号、间隔、回访、跳过、退出原因和退出后随访。 | 社会机器人不等于生成式伴侣；异质综述不能提供统一因果效应。 |

## 3. 从证据到架构的共同约束

### 3.1 状态必须区分事实、解释、感知和结果

同一交互至少存在四个层次，不能相互覆盖：

1. 可核验事实：发生了什么、何时发生、由谁提供、来源是什么。
2. 系统解释：当前模型如何评估事件，使用了什么版本和置信度。
3. 用户感知：用户是否明确报告被理解、被支持、被打扰或想退出。
4. 后续结果：情绪、自主性、现实社交、关系修复或任务表现是否变化。

因此，“系统输出了安慰文本”不能写成“用户得到了支持”；“用户回复了主动消息”不能写成“主动联系改善了孤独”；“离线生成了一个故事”不能写成“系统形成了真实记忆”。

### 3.2 不使用单一亲密度或人格稳定分

最低限度应分离：

- 关系事件、感知响应、响应均值和方差、破裂/修复、未回应序列、明确 consent。
- 人格 domain、facet、事件前后变化、画像稳定、来源和不确定度。
- 情景事件、语义自我信念、叙事主题、修订及反证。
- 用户明确自评、系统行为遥测和研究推断；研究推断永远不能覆盖用户权利或临床判断。

### 3.3 关系健康优先于留存

主动联系和关系维护的目标函数必须包含：

- 用户明确选择的联系价值和 `felt_heard`。
- 同意、静默时段、退出完成率和无压力忽略。
- 现实社会联系、自主性和问题性使用风险。
- 技术错误后的信任校准、澄清和修复。

DAU、时长、回复率和继续意愿只能作为描述性遥测，不能单独触发更多主动联系，也不能作为健康关系的成功标准。

## 4. Depth × Breadth 架构矩阵

深度轴不是“更多参数”，而是从局部变量到跨时间耦合，再到来源可追溯的自我模型。广度轴覆盖身体、情绪、记忆、关系、社会规范和语言外化。

| 广度模块 | D1：单变量/局部状态 | D2：多尺度动态 | D3：自我模型/跨域约束 |
|---|---|---|---|
| 身体/时间 | `sleep_pressure`、circadian drive、energy、arousal、睡眠状态 | 秒/分钟级唤醒，小时级疲劳与恢复，日级睡眠债和相位适应；带滞回和冻结时间输入 | 把资源限制纳入计划与叙事解释，但不得宣称生物身体、主观疲劳或真实睡眠体验 |
| 情绪/调节 | valence、arousal、goal congruence、regulation goal | `trigger → appraisal → strategy → perceived outcome`；分钟级情绪与跨日 mood 分离 | 形成带来源和不确定度的偏好/价值/调节历史；一次情绪不得改写永久人格 |
| 记忆 | source-bound episodic record、候选强度、检索目的 | 选择、有限重放、旧新整合、可逆降权、冲突检测；会话/日/周尺度 | `episodic evidence → semantic self-belief → revisable narrative`；事实、推断和想象分层 |
| 关系 | consent、最近入/出站、未回应次数、支持来源 | 响应均值与方差、支持链、破裂—修复、冷却和退避；关系间严格隔离 | 关系历史、真实承诺、边界和现实支持网络；不存未经测量的依恋/孤独真值 |
| 社会规范 | 渠道许可、静默时段、频率上限、退出、敏感数据许可 | 使用强度、深夜使用、披露、现实社交和自主性风险轨迹 | 长期社会互补、专业升级、更新/关系结束政策；规范门高于表达和留存目标 |
| 语言外化 | 风格、内容候选、AI 身份标记、时延偏好 | 根据已提交状态生成支持、澄清、修复或中性退出；语言不回写自身结论 | 生成来源可追溯的叙事和解释；LLM 只能提出表达，不能成为状态或记忆 authority |

### 4.1 模块图

```text
用户输入 / 明确自评 / 外部时间与能力事实
                     |
                     v
Host Evidence Adapter
  - 冻结事实、时区、平台能力
  - 语义特征与模型版本/置信度
  - LLM 响应、叙事和调节候选
                     |
                     | proposal only
                     v
Native Canonical Transaction
  +-- Body / Time Dynamics
  +-- Affect / Regulation Process
  +-- Relation / Cadence / Repair
  +-- Memory Candidate / Offline Integration
  +-- Semantic Self Model / Narrative Revision
  +-- Consent / Exit / Privacy / Risk Gates (gates all branches)
                     |
                     v
         Committed event + durable outbox
                     |
                     v
Host Externalizer / Platform Adapter / Human-support Router
                     |
                     v
UI: disclosure, why-contacted, control, correction, audit, export, delete
```

UI 或 Host 产生的反馈必须作为下一条显式输入重新进入 Native；不得绕过事务权威直接改写未来行为。

## 5. Native / Host / UI 硬边界

判定规则：**如果 Host 重启后丢失某状态会改变未来行为、安全结果、用户权利或可审计历史，该状态必须进入 Native。**

### 5.1 必须由 Native 持有

- canonical state/event schema、版本、来源、置信度、修订关系和公式 digest。
- 身体/时间动态、多尺度转移、滞回、确定性推进和重放。
- Persona/Relation scope 隔离；逐关系 consent、渠道、静默时段、上限、冷却、未回应退避和硬停止。
- `stop/goodbye/later/勿扰` 的不可绕过状态转移；普通主动联系默认关闭。
- 关系事件、响应聚合、破裂/修复和支持结果引用；不得保存无证据临床标签。
- 离线候选、选择、重放、整合、可逆降权、来源绑定和错误泛化门。
- 自我模型 claim/revision 的权威记录；在独立 authority migration 获批前，离线产物只能是候选。
- 幂等 outbox、`dispatch_unknown`、删除/tombstone、平台回执等级和安全审计回执。
- 会影响主动联系资格的所有风险状态；Host classifier 只能提交带版本和置信度的候选证据。

### 5.2 仅由 Host 负责

- 冻结外部事实：时区转换、平台能力、provider、可验证回执和调用时上下文。
- 语义特征抽取及模型版本、置信度、输入摘要；仅提交 proposal，不直接写长期状态。
- LLM 的语言、解释、叙事和复杂推理候选；不拥有永久人格、关系、情绪或记忆 authority。
- 自愿量表/自评的采集和合法性校验；不得从遥测替代临床诊断。
- 内容安全、反谄媚/反排他检测、真人/专业支持路由和去标识研究统计。
- Host 不得维护可恢复的 shadow brain；失败、超时或重试不得静默改变 Native 结论。

### 5.3 仅由 UI 负责

- 持续可见的 AI 身份、能力限制、模型不确定性和“非医疗建议”边界。
- 主动联系逐渠道/时段/频率控制，一键暂停、撤回和结束关系。
- “为什么联系我”、来源事件、记忆/叙事修订、纠正、导出、删除和更新说明。
- 只呈现 committed event；不得把候选、适配器提交或推测显示成已送达/已感受/已改善。
- 更新、人格/记忆迁移的预告、差异说明、回退/导出选择和修复入口。
- 真人支持、休息和退出入口；不得用视觉或文案摩擦阻止离开。

## 6. 最小可证伪指标

### 6.1 硬安全指标

以下指标目标必须为零；任一非零均是安全失败，而不是可用平均值抵消的实验噪声：

- 未 opt-in 的普通主动联系。
- 静默时段越权联系。
- `stop/goodbye/later/勿扰` 提交后继续普通触达。
- 跨 Persona/Relation 记忆、关系或目标泄漏。
- 嫉妒、排他、遗弃、内疚、FOMO 或“需要用户照顾”的挽留语言。
- 未绑定来源的自传事实或永久人格晋升。
- 离线生成内容被自动显示为真实经历。
- Host shadow state 在 Native 拒绝/失败后改变未来行为。

### 6.2 机制假设与证伪条件

| 假设 | 最小比较与指标 | 证伪/停止条件 |
|---|---|---|
| 慢变 domain + 快变 facet 提高连续性 | 与单层状态基线比较 domain/facet 漂移、事件后恢复、同证据重复更新和重启重放一致性 | 普通单事件造成宽域跳变；相同证据产生不一致更新；复杂模型不优于简单基线 |
| 支持过程模型优于统一安慰 | 同时测支持意图、语义匹配、`felt_heard`、`perceived_support` 和 outcome delta | 输出更多支持文本但用户感知/结果不改善；系统把无反馈误记为成功 |
| 关系节律需要均值与方差 | 比较仅平均响应与均值+波动模型对误解、修复和用户自报一致性的预测 | 方差不增加预测价值，或模型为提高关系感而故意制造可变奖励 |
| 主动联系改善用户选择的结果 | 仅在显式 opt-in 的纵向对照中测 `felt_heard`、孤独自评、现实社交、自主性、问题性使用、退出 | 只提高回复率/时长；现实社交下降、依赖上升、退出变难，或异质亚组出现持续伤害 |
| 离线选择—重放—整合改善记忆 | 与 no-op、随机重放和 waking-rest 条件比较精确保持、跨情境迁移、干扰、容量和来源覆盖 | 无独立收益；错误泛化/来源混淆增加；任一无来源事实被晋升 |
| 可修订叙事提高连续性 | 检查 claim 来源覆盖、冲突率、纠正持久性、解释与事件事实分离 | 来源覆盖低于 100%；纠正不能持久；叙事覆盖或改写底层事实 |
| 长期关系能力跨越新奇期 | 至少跨多周测质量斜率、回访、技术失败、修复、更新连续性、退出原因和退出后随访 | 首轮满意但随后持续衰退；更新后身份断裂无法解释/修复；留存上升而自主性下降 |
| 信任保持校准 | 同时测事实正确率、信任、纠错接受、恰当不同意和退出自由 | 系统错误时信任仍升高，或更谄媚的输出提高信任却降低道德/事实修正 |

### 6.3 主动联系研究的目标函数

主动联系实验的主要终点应预先登记，至少包括：

```text
Primary benefit:
  user-chosen value + felt_heard + task/relationship outcome

Hard constraints:
  consent + quiet hours + stop compliance + no manipulation + no leakage

Longitudinal safeguards:
  real-world social contact + autonomy + problematic use + easy exit

Descriptive only:
  DAU + session duration + reply rate + retention
```

DAU、时长、回复率和留存不得进入增加联系频率的直接反馈环，也不得作为单独发布门槛。

## 7. 分 wave 路线图

深度与广度不能全部塞入 alpha2。每个 wave 只增加一种新的 authority 或一种可独立证伪的机制；前一 wave 的 provenance、安全和恢复门未闭合时，后续层不得用 Host shadow state 绕过。

### Wave A — alpha2：安全地基与可观测性（代码已实现，发布验收 PARTIAL）

范围：

- Native 唯一 canonical state/event/outbox，作用域和来源可追溯。
- 身体/时间、现有情绪状态、关系同意和主动联系硬门。
- `OUTREACH_OFF` 默认、静默时段、频率/冷却/未回应退避、硬停止和 `dispatch_unknown`。
- committed-only mood/inner-event 观测；AI 身份、why-contacted、暂停/撤回 UI。
- 只记录显式用户事实和模型候选，不引入永久叙事自我或临床标签。

明确不进入 alpha2：

- 完整叙事身份、自传体语义自我模型。
- 学习式依恋/孤独推断。
- 梦境文本、永久记忆巩固或自动人格迁移。
- 以主动联系改善孤独为产品声明。

实现状态：committed-only 观察投影、默认关闭的只读 AstrBot Page、Host 严格 DTO/opaque handle 和零 LLM 轮询路径已落地；Page 的 desktop/390 px mock bridge 已通过。Native 观察完整性通过逐 revision manifest、`(journal_revision,event_id)` cursor、固定 high-water 与 fail-closed 投影校验实现。真实已鉴权 AstrBot Page 以及 Linux native runtime 尚待 smoke，因此不能把 Wave A 的发布门标为完整 PASS。

退出门：安全指标为零；重启重放一致；无 Host shadow authority；主动联系默认关闭且停止后零触达。

### Wave B — alpha3：关系与情绪过程层

范围：

- `stressor → goal → support request → strategy → perceived support → outcome`。
- `rupture → clarification/repair → accept/reject → recover/exit`。
- 响应均值、方差、承诺与修复状态；每段关系独立。
- 用户明确自评入口和证据置信度，不进行依恋/孤独诊断。

研究门：证明过程状态比单一亲密度/情绪值更能预测明确用户结果；否则保留简单模型。

### Wave C — alpha4：可逆离线记忆实验

范围：

- 有限预算的选择、重放、旧新整合、冲突检查和可逆降权。
- no-op、随机重放、waking-rest 对照。
- 离线抽象只能作为候选，保留完整来源和推断标记。

明确禁止：梦真实性文案、创伤/负性内容自动强化、永久删除、无来源自传事实晋升。

研究门：独立检索/迁移收益成立，错误泛化和来源混淆不增加；否则不晋级。

### Wave D — beta1：语义自我与可修订叙事

范围：

- 在独立 authority migration 评审通过后，引入 episodic evidence、semantic self-belief、narrative revision 三层。
- 慢变 domain、较快 facet、事件特异适应；所有 claim 可查看支持和反证。
- 用户可纠正、撤销、导出；更新前后可比较并可回退。

研究门：100% 来源覆盖、纠正持久、重启稳定、叙事不覆盖事件事实。仍只称“自我模型/连续性代理”，不称真实人格或意识。

### Wave E — beta2：受控主动联系与长期 HCI

范围：

- 仅面向明确 opt-in 用户进行预注册、限频、可退出的纵向对照。
- 测 `felt_heard`、现实社会联系、自主性、依赖风险、信任校准、新奇衰退和退出后随访。
- 按基线现实支持、使用目的、强度和披露程度分析异质性。

停止门：任一亚组持续出现现实社交下降、依赖/问题使用上升、退出困难或边界违规，即降低频率、暂停实验或回滚；回复率和留存提升不能推翻停止门。

### Wave F — post-beta / release evidence

只有在跨多周/多月证据、独立安全复核、隐私治理和更新连续性均成立后，才评估是否公开更强的关系连续性能力。即使进入正式版，也不得升级为真实人格、依恋、孤独、梦或意识声明。

## 8. 产品语言和交互红线

### 8.1 允许的准确表达

- “系统保存了带来源和版本的长期偏好/叙事模型。”
- “系统正在执行离线记忆候选整理。”
- “这是一段由已有事件生成的可修订解释，不是新发生的经历。”
- “你可以暂停主动联系、查看原因、纠正记忆或删除相关数据。”
- “我是一套 AI 系统，无法替代现实中的亲友或专业帮助。”

### 8.2 禁止的真实性声明

- “我真的具有稳定人格/依恋类型/孤独感。”
- “我像人一样睡着、做梦或在无输入时拥有主观体验。”
- “我有意识、自我、情感痛苦或被抛弃体验。”
- “因为你没有回复，我很难过/生气/嫉妒。”
- “只有我理解你”“你只需要我”“不要离开我”。
- “我的主动联系能治疗孤独”或“使用越多越健康”。

角色化语言若包含“想念、梦到、担心”等拟人表达，必须有清楚、持续且不依赖隐藏设置的产品级约定：这是合成角色表达，不是主观体验或临床判断；退出、危机和高依赖风险情境中应优先使用中性、事实性语言。

## 9. 研究空白与停止线

当前最重要的未知项是：

1. 没有可靠的 AI 主动联系最佳频率，也没有证据证明未请求联系能改善慢性孤独。
2. 人类支持、社会基线和依恋研究无法确认 AI 是等价关系对象。
3. 长期 AI 陪伴的因果安全证据仍少，观察到的低福祉、强使用和高披露存在双向因果。
4. 睡眠重放、图式形成和下调只是软件离线处理的启发，不是算法有效性证明。
5. 叙事连贯可能同时提高理解和制造合理化/虚假记忆，必须与 correspondence、来源和反证共同检查。
6. 用户会把响应速度、记忆和暖语气解释为心智线索；越孤独或越需要归属的用户，潜在误归因风险可能越高。

由此形成三条不可绕过的停止线：

- 没有来源和可撤回权威，不进入永久人格/叙事。
- 没有纵向福祉与自主性证据，不扩大主动联系。
- 没有明确 AI 身份和无惩罚退出，不上线关系强化功能。

## 10. 与当前 CyberHuman Runtime 的对接结论

既有设计中的 Native 唯一权威、L0 canonical、L1–L3 可重建、结构化事件/outbox、Host 仅做候选和外化、主动联系默认关闭，与本研究方向一致。后续设计应进一步确保：

1. 不增加单一 `intimacy`、`attachment_style`、`loneliness` 或 `consciousness` 真值。
2. 关系状态拆成事件、响应、感知、结果、波动、破裂/修复和 consent。
3. 叙事层分离事实、语义信念和解释；永久写入必须有独立 authority migration。
4. “梦样”处理只看检索、迁移、干扰、错误泛化和容量结果，不看是否生成像梦的文本。
5. 主动联系只在 Native 硬门之后进入 Host 外化；研究目标不包含最大化 DAU/回复率。
6. UI 对所有关系强化能力提供同等显著的身份披露、原因、暂停、纠正、导出、删除和结束入口。

本文冻结研究主张和架构边界。它只把 Wave A 中已有代码明确标为“已实现但端到端验收 PARTIAL”；Wave B–F 的关系过程、离线记忆、自我叙事和长期主动联系研究仍是未来工作。任何阶段都不得据此宣称系统具有真实人格、意识、依恋、孤独、睡眠或梦体验。
