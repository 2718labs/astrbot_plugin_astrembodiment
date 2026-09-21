# AstrEmbodiment

AstrEmbodiment 是 AstrBot 的 Rust 原生人格情绪、语义评价和确定性身体时钟核心。Genesis 与 SeedCode 为 Persona 建立身份起点；普通对话通过类型化入口提交输入、语义评价和实际投递结果。

当前源码版本统一为 `1.1.0`（Rust、Python/wheel 与插件元数据）。该版本保留现有无头人格情绪、语义评价、确定性身体时钟核心，并安全兼容精确识别的历史旧 schema。版本去除 alpha 标记仅表示当前源码身份定版，不代表已重建 `1.1.0` 安装包，也不代表 AstrBot 实机安装、对话、重启或生产发布验收；这些仍须以对应外部验证回执为准。生产发布授权门禁仍为 **NO_GO**，既有 alpha5 与更早交付产物保持不变。

## 能力与边界

- 保留 Persona Genesis、SeedCode、核心情绪状态、语义评价，以及普通对话的投递结算。
- Persona 独立身体时钟使用随包冻结的 IANA 2026c 数据；时区、日程、单调性和重启状态由原生类型化接口管理。
- Native 与 Python 包装器仅开放契约规定的 19 个方法，包括无数据库能力的 `compile_core_host_request_v1`。
- 历史 kind-8 DTO 与 codec 仅用于只读回放、摘要核验和迁移验证，不提供事件提交路径。
- 主动消息、接触意图、目标/密钥访问、派发和发送执行入口已退休；Pages、Observatory、旧实时心智入口和 AstrCyberHuman 依赖不属于本包。

完整边界见 [Task 11/12 契约](docs/superpowers/plans/2026-09-05-task11-12-core-boundary-embodiment-clock/contracts/retired-surface-release.md)。既有配置、数据库、密文和密钥文件不属于源码发布工具的写入范围。

## 安装与配置

取得已通过同源 Windows x64 与 Linux x86_64 实机导入验证的 universal ZIP 后，使用 AstrBot 插件管理页安装。要求 AstrBot `>=4.16,<5`、CPython 3.12+；Linux 使用兼容 glibc 的 x86_64 wheel。

命令 `ae` 显示核心状态，`ae_seed` 查询或生成并保存 SeedCode。首次使用先检查原生加载、Genesis 与普通对话投递，再观察身体时钟在重启后的连续性。

配置字段以 [_conf_schema.json](_conf_schema.json) 为准，包括 `runtime_envelope`、`native_data_dir`、`model_settings` 和 `seed_code`。SeedCode 由 AstrBot 保存接口持久化；不要在安装或升级时删除已有 SeedCode、数据库、密文或密钥。退休配置字段不再提供主动执行能力。

升级兼容以保全现有数据为前提：仅精确识别历史提交 `1774023` 或 `ee10648` 中的已知旧表结构，并且仅在对应 upgrade 表、备份表与 authority 表均为空、且不存在 `AE-LSU1` 升级历史时，才允许在事务内兼容到当前结构。普通业务历史数据完整保留，此门禁不要求整个数据库为空。upgrade 表非空、证据不足或结构未知时，安装必须保全原库并拒绝继续，不得通过删除、清空或重建数据库绕过门禁。

## 源码检查与构建

源码准备阶段运行：

```text
cargo check -p astrembodiment-core --offline -j 1
python scripts/validate_kit.py
python scripts/scan_core_boundary.py
python -m pytest tests/test_release_contracts.py tests/test_static_contracts.py
git diff --check
```

当前新门禁的重点检查可用 `-k 'test_alpha4 or release_versions or initializer_and_packager or config_schema or formula_and_runtime or region_layout or self_action_has'` 选择。上述发布测试文件中的旧 readiness/wake 调用、占位二进制成功打包和旧 manifest 结构断言尚待迁移；全文件通过与最终归档验收仍是后续门禁，不能用这组重点检查替代。

最终构建必须在所有源码和回归迁移提交完成后的干净工作树执行，以同一个完整 40 位 Git SHA 替换下方 `SOURCE_SHA`。构建输出使用新的目录；构建脚本把 SHA 注入 Native，同时将编译身份写入 wheel 的 `astrembodiment_core/build_identity.json` 并更新 RECORD。

Windows：

```text
python scripts/package_plugin.py --build-native --source-sha SOURCE_SHA --output dist/1.1.0-win --target x86_64-pc-windows-msvc
python scripts/package_plugin.py --verify-wheel dist/1.1.0-win/astrembodiment_core-1.1.0-cp312-abi3-win_amd64.whl --output dist/1.1.0-win-import.json
```

Linux 构建（可使用受控交叉工具链，但导入必须发生在实际 Linux x86_64 上）：

```text
python scripts/package_plugin.py --build-native --source-sha SOURCE_SHA --output dist/1.1.0-linux --target x86_64-unknown-linux-gnu --compatibility manylinux_2_17 --zig
python scripts/package_plugin.py --verify-wheel PATH_TO_LINUX_WHEEL --output dist/1.1.0-linux-import.json
```

构建入口支持 `--maturin PATH`。实际 wheel 名称以构建输出为准；Linux 导入所在工作树也须处于同一干净 SHA。实机凭据记录解释器、平台、Native 方法集合、wheel/二进制哈希及导入的构建身份。

两侧导入通过后才可组装：

```text
python scripts/package_plugin.py --source-sha SOURCE_SHA --native-wheel PATH_TO_WINDOWS_WHEEL --native-wheel PATH_TO_LINUX_WHEEL --import-receipt dist/1.1.0-win-import.json --import-receipt dist/1.1.0-linux-import.json --output dist/astrbot_plugin_astrembodiment-1.1.0-universal.zip
```

现有 `astrembodiment_core/_bundled/manifest.json` 记录双平台 wheel、二进制、同源身份和导入凭据。归档生成后仍需对精确 ZIP 完成 artifact parity、保护样本哈希核验和独立审查。导入凭据是构建证据记录，不是签名或发布授权。

## 许可

[AGPL-3.0-or-later](LICENSE)。版本历史见 [CHANGELOG.md](CHANGELOG.md)。
