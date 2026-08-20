# 仓库地图

```text
astrbot_plugin_astrembodiment/
├── main.py                         AstrBot 唯一 Star 入口
├── astr_embodiment/                Python 薄宿主
├── python/astrembodiment_core/     native wheel Python package
├── crates/
│   ├── ae-fixed/                   定点数与确定性数学
│   ├── ae-contracts/               closed types
│   ├── ae-authority/               来源权限与因果凭据
│   ├── ae-attention/               微型注意力/荷载装配
│   ├── ae-neurofield/              16K 神经场/动态图
│   ├── ae-mechanics/               能量、本构、返回映射
│   ├── ae-renorm/                  多尺度 restriction/prolongation
│   ├── ae-agent/                   world model/action competition
│   ├── ae-continuum/               journal/snapshot/delta/replay
│   ├── ae-store/                   SQLite repository
│   ├── ae-runtime/                 唯一 writer/composition root
│   └── ae-pyo3/                    Python extension
├── model/                          FormulaProfile 配置
├── docs/                           产品、架构、公式与工程文档
├── adr/                            决策记录
├── tests/                          scenario/gauntlet/contract
└── scripts/                        构建、打包、资源测试
```

依赖方向：

```text
ae-fixed
   ↓
ae-contracts
   ↓
ae-authority  ae-attention  ae-neurofield  ae-mechanics  ae-renorm
          \       |              |              |          /
                         ae-agent
                            ↓
                      ae-continuum
                            ↓
                         ae-store
                            ↓
                        ae-runtime
                            ↓
                         ae-pyo3
```

低层 crate 不能依赖 AstrBot、Python 或 SQLite。
