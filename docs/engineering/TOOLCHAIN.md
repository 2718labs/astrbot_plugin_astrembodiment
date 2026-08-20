# 工具链基线

核验日期：2026-08-15。

## AstrBot

- Python：3.12+
- 插件仓库推荐命名：`astrbot_plugin_*`
- 必需入口：`main.py`
- 插件元数据：`metadata.yaml`

官方参考：

- https://github.com/AstrBotDevs/AstrBot
- https://github.com/AstrBotDevs/AstrBot/wiki/en-dev-star-plugin-new
- https://github.com/AstrBotDevs/AstrBot/wiki/en-dev-star-guides-simple

## Rust/Python 边界

- PyO3：0.29.0
- Stable ABI feature：`abi3-py312`
- Maturin：1.14.1

官方参考：

- https://pyo3.rs/v0.29.0/
- https://www.maturin.rs/
- https://pypi.org/project/maturin/1.14.1/

## 首个开发机命令

```bash
rustup update stable
python -m venv .venv
source .venv/bin/activate   # Windows 使用 .venv\\Scripts\\activate
python -m pip install -r requirements-dev.txt
cargo check --workspace
maturin develop
python -c "import astrembodiment_core; print(astrembodiment_core.health())"
python -m compileall -q main.py astr_embodiment python
```

当前开发包生成环境未安装 Rust，因此不能把骨架视为已经通过 `cargo check`。
