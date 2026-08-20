# ADR-004：不建设长期语义记忆

## 状态

Accepted.

## 决策

AstrEmbodiment 不保存原始文本、摘要、embedding 或长期用户事实画像。只保存量化事件、行动/结果因果、神经与 residual 状态。

## 产品表达

她不能记起发生过什么，但可以保留那些经历塑造出的性情、容忍度、边界、伤痕与修复。

## 与 AstrBot 的关系

AstrBot 当前会话历史仍供语言模型完成正常对话；AstrEmbodiment 不额外建设第二套文本记忆。
