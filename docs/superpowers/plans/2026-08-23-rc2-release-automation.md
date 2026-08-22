# RC2 版本、质量门禁与标签发布实施计划

> **给执行代理：** 必须逐任务使用 executing-plans 与 test-driven-development；所有步骤使用复选框跟踪。

**目标：** 将项目版本一致地升级为 1.0.0-rc2，并在 GitHub 上建立只读 CI 与由既有版本标签触发的双平台自动 Release。

**架构：** 受测试保护的 release contract 脚本从 metadata、pyproject、Cargo、CHANGELOG 和 tag 推导唯一发布身份。CI 仅验证源代码和打包；release workflow 仅对已存在 tag 使用短范围 contents: write 创建或更新 GitHub Release，绝不自行打 tag 或合并 PR。

**技术栈：** GitHub Actions、GitHub CLI、Python 3.12、pytest、ruff、maturin、Cargo、Windows/Linux GitHub-hosted runners。

---

## 文件职责和写入边界

| 文件 | 职责 |
| --- | --- |
| metadata.yaml | AstrBot 可见版本与产品描述 |
| pyproject.toml | Python PEP 440 版本 |
| Cargo.toml 与 Cargo.lock | Rust workspace 版本 |
| CHANGELOG.md | RC2 已实现内容与发布边界 |
| README.md | 面向用户的 RC2 能力、限制、CI/release 使用说明 |
| scripts/validate_release.py | 无网络的版本、tag、归档命名验证器 |
| tests/test_release_contracts.py | 版本、脚本、workflow 权限与关键门禁回归 |
| .github/workflows/ci.yml | PR/push 只读质量、构建与打包验证 |
| .github/workflows/release.yml | 既有 tag 的受控自动 GitHub Release |
| .github/dependabot.yml | GitHub Actions 依赖的按周 PR 更新建议 |

本计划不改 15 维路由、native 表达档案、bridge 或 main.py 的同轮注入逻辑；那些只属于 2026-08-23-rc2-native-expression.md。

### Task 1：写版本与发布脚本的 RED 测试

**文件：**

- 修改：tests/test_release_contracts.py
- 创建：scripts/validate_release.py

- [ ] **步骤 1：添加版本三元组和 tag 验证测试**

在 test_release_contracts.py 增加常量：

~~~python
RELEASE_VERSION = "1.0.0-rc2"
PYTHON_RELEASE_VERSION = "1.0.0rc2"
RELEASE_TAG = "v1.0.0-rc2"
~~~

添加三类 subprocess 测试：

~~~python
def test_release_validator_accepts_matching_rc_tag(tmp_path: Path) -> None:
    result = run_release_validator("--tag", "v1.0.0-rc2")
    assert result.returncode == 0, result.stderr

def test_release_validator_rejects_tag_with_other_version(tmp_path: Path) -> None:
    result = run_release_validator("--tag", "v9.9.9-rc2")
    assert result.returncode != 0
    assert "tag does not match metadata version" in result.stderr

def test_release_validator_rejects_archive_with_wrong_name(tmp_path: Path) -> None:
    wrong_archive = tmp_path / "wrong.zip"
    wrong_archive.write_bytes(b"zip")
    result = run_release_validator("--tag", RELEASE_TAG, "--archive", str(wrong_archive))
    assert result.returncode != 0
    assert "archive filename" in result.stderr
~~~

测试还必须断言 metadata、Python、Cargo、Cargo.lock 中的 workspace package 版本和 CHANGELOG 的 RC2 标题均使用唯一版本。

- [ ] **步骤 2：运行测试并确认 RED**

运行：

~~~text
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
.venv\Scripts\python.exe -m pytest tests/test_release_contracts.py -q -o cache_dir=G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\pytest-rc2-release
~~~

预期：缺少 validate_release.py、版本仍为 rc1 或 CHANGELOG 还没有 RC2 节时失败。

- [ ] **步骤 3：实现无网络 release validator**

创建 scripts/validate_release.py。它必须：

1. 用标准库读取 metadata.yaml 的 version 行、pyproject.toml 的 project.version 和 Cargo.toml 的 workspace.package.version。
2. 把 metadata 的 -rcN 转换为 Python 的 rcN，其他普通版本保持不变。
3. 读取 CHANGELOG.md，要求存在精确的 RC2 标题。
4. 接受必填 --tag，要求等于 v 加 metadata version。
5. 可选接受 --archive，要求文件存在且 basename 精确等于由 release_version 拼入的 astrbot_plugin_astrembodiment-版本号-win_linux_x86_64.zip；RC2 的实际值为 astrbot_plugin_astrembodiment-1.0.0-rc2-win_linux_x86_64.zip。
6. 失败时向 stderr 输出固定、无私有内容的原因并返回 1；成功打印版本与 tag 并返回 0。

核心入口形式：

~~~python
def validate(tag: str, archive: Path | None) -> None:
    release_version = read_metadata_version(ROOT / "metadata.yaml")
    if tag != f"v{release_version}":
        raise ValidationError("tag does not match metadata version")
    assert_python_version(release_version, read_pyproject_version())
    assert_cargo_version(release_version, read_cargo_version())
    assert_changelog(release_version)
    if archive is not None:
        assert_archive_name(release_version, archive)
~~~

- [ ] **步骤 4：运行版本测试并确认 GREEN**

运行：

~~~text
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
.venv\Scripts\python.exe -m pytest tests/test_release_contracts.py -q -o cache_dir=G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\pytest-rc2-release
~~~

预期：新 validator 的有效 tag 通过，所有错误 tag/归档场景可预测失败。

- [ ] **步骤 5：提交发布身份验证器**

~~~text
git add scripts/validate_release.py tests/test_release_contracts.py
git commit -m "test: enforce rc2 release identity"
~~~

### Task 2：先用静态测试锁住 CI 与 release 约束

**文件：**

- 修改：tests/test_release_contracts.py
- 修改：.github/workflows/ci.yml
- 创建：.github/workflows/release.yml
- 创建：.github/dependabot.yml

- [ ] **步骤 1：写 workflow RED 测试**

不依赖第三方 YAML 解析库，读取文本并断言关键语义。测试必须验证：

~~~python
def test_ci_is_read_only_and_has_all_quality_gates() -> None:
    ci = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
    assert "contents: read" in ci
    assert "ruff format --check" in ci
    assert "ruff check" in ci
    assert "cargo fmt --all -- --check" in ci
    assert "cargo clippy --workspace --all-targets" in ci
    assert "cargo test --workspace --locked" in ci
    assert "windows-latest" in ci
    assert "ubuntu-latest" in ci
    assert "scripts/package_plugin.py" in ci

def test_release_runs_only_for_existing_tags_with_scoped_write_permission() -> None:
    release = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
    assert "tags:" in release and "'v*'" in release
    assert "contents: write" in release
    assert "git tag " not in release
    assert "gh release create" in release
    assert "--prerelease" in release
    assert "scripts/validate_release.py" in release
~~~

再断言 CI 中不存在 contents: write、gh release create 或 git tag。

- [ ] **步骤 2：运行测试并确认 RED**

运行：

~~~text
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
.venv\Scripts\python.exe -m pytest tests/test_release_contracts.py -q -o cache_dir=G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\pytest-rc2-workflow
~~~

预期：现有最小 ci.yml 缺 ruff、Cargo、双平台打包，release.yml 不存在，测试失败。

- [ ] **步骤 3：按固定 job 划分实现 CI**

将 ci.yml 改为 contents: read，并加入 concurrency。使用已由一手 GitHub 文档核实的完整 commit SHA 固定 actions/checkout、actions/setup-python、actions/upload-artifact、actions/download-artifact。

CI 至少有以下 jobs：

~~~text
python-quality
rust-quality
native-wheel (matrix: windows-latest, ubuntu-latest)
package-contract (needs: native-wheel, python-quality, rust-quality)
~~~

python-quality 安装 requirements-dev.txt，执行 ruff format --check、ruff check、py_compile、pytest。rust-quality 执行 cargo fmt --all -- --check、cargo clippy --workspace --all-targets --locked -- -D warnings、cargo test --workspace --locked。native-wheel 在每个平台用 maturin build --release --locked --out dist 生成 wheel 并上传。package-contract 下载两个 artifact，在 Linux runner 用 scripts/package_plugin.py 组装 RC2 ZIP，运行 validate_release.py，并检查 ZIP manifest 和 SHA-256。

- [ ] **步骤 4：实现 tag 驱动 release workflow**

release.yml 只对：

~~~yaml
push:
  tags:
    - 'v*'
workflow_dispatch:
  inputs:
    tag:
      required: true
      type: string
~~~

触发。workflow_dispatch 的第一步把输入赋给 TAG，执行 git rev-parse "refs/tags/$TAG" 验证标签确实存在，随后 checkout 该 tag。它重跑 CI 的格式、lint、测试和双平台 wheel job，组装 ZIP 后执行 validate_release.py --tag "$TAG" --archive "$ARCHIVE"。

最终 job 使用 GitHub 自动注入 token 与 GitHub CLI：

~~~bash
if gh release view "$TAG" >/dev/null 2>&1; then
  gh release upload "$TAG" "$ARCHIVE" "$ARCHIVE.sha256" --clobber
else
  if [[ "$TAG" == *-rc* ]]; then
    gh release create "$TAG" "$ARCHIVE" "$ARCHIVE.sha256" --title "$TAG" --generate-notes --prerelease
  else
    gh release create "$TAG" "$ARCHIVE" "$ARCHIVE.sha256" --title "$TAG" --generate-notes
  fi
fi
~~~

只有 release job 有 contents: write。workflow 中不能出现 git tag、git push、PR merge、Marketplace API、secret 回显或自动合并。

dependabot.yml 仅为 github-actions 每周创建更新建议 PR，不赋予写 token，不自动合并。

- [ ] **步骤 5：运行 workflow contract 测试并确认 GREEN**

运行：

~~~text
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
.venv\Scripts\python.exe -m pytest tests/test_release_contracts.py -q -o cache_dir=G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\pytest-rc2-workflow
~~~

预期：工作流文本满足 read/write 边界、必要门禁、双平台构建和标签约束；没有实际触发 GitHub Actions 或发布。

- [ ] **步骤 6：提交工作流**

~~~text
git add .github/workflows/ci.yml .github/workflows/release.yml .github/dependabot.yml tests/test_release_contracts.py
git commit -m "ci: add quality gates and tagged releases"
~~~

### Task 3：以 RED 测试升级所有 RC2 版本与产品表述

**文件：**

- 修改：metadata.yaml
- 修改：pyproject.toml
- 修改：Cargo.toml
- 修改：Cargo.lock
- 修改：CHANGELOG.md
- 修改：README.md
- 修改：tests/test_release_contracts.py
- 修改：tests/test_runtime_integration.py

- [ ] **步骤 1：把既有 RC1 常量测试先改成 RC2 期望**

将 release-contract 测试中的版本、归档名、wheel 名与 native health 期望改为 RC2。将 runtime integration 中加载器的 version 期望从 1.0.0-rc1 改为 1.0.0-rc2。

新增 README 断言，要求文本同时出现“同轮”或“当前轮”、“15 维”、“原生表达投影”，以及“不等于有意识”或等价限制；新增 CHANGELOG 断言要求 RC2 段列出表达投影、观测 v2 和自动化发布边界。

- [ ] **步骤 2：运行测试并确认 RED**

运行：

~~~text
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
.venv\Scripts\python.exe -m pytest tests/test_release_contracts.py tests/test_runtime_integration.py -q -o cache_dir=G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\pytest-rc2-version
~~~

预期：源文件仍声明 RC1，因此版本、README 和 CHANGELOG 断言失败。

- [ ] **步骤 3：实施统一版本和文档修改**

修改值：

~~~text
metadata.yaml: version: "1.0.0-rc2"
pyproject.toml: version = "1.0.0rc2"
Cargo.toml: version = "1.0.0-rc2"
~~~

用 cargo metadata --locked 或 cargo check --locked 更新 Cargo.lock 中所有 workspace package 的版本，不手工搜索替换 lockfile。

在 CHANGELOG 的 Unreleased 下新增 [1.0.0-rc2]，只列已实现的 15 维原生路由、确认后同轮表达投影、v2 observatory、CI 与标签发布路径；注明远端 tag、Release、Marketplace 仍未在本次源码修改中执行。

README 的首屏产品介绍改为中文产品语言，说明“用户话语 -> 15 维证据 -> native commit -> 当前轮表达投影”的闭环，同时明确它是类情感计算，不等于真实意识；加入 CI badge 与“推送 v1.0.0-rc2 标签将触发 GitHub prerelease”的准确说明。

- [ ] **步骤 4：运行版本和文档 GREEN**

运行：

~~~text
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
.venv\Scripts\python.exe -m pytest tests/test_release_contracts.py tests/test_runtime_integration.py -q -o cache_dir=G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\pytest-rc2-version
.venv\Scripts\python.exe scripts/validate_release.py --tag v1.0.0-rc2
~~~

预期：源码版本、锁文件、文档和本地 tag 形状全部一致；命令不创建 tag。

- [ ] **步骤 5：提交 RC2 身份**

~~~text
git add metadata.yaml pyproject.toml Cargo.toml Cargo.lock CHANGELOG.md README.md tests/test_release_contracts.py tests/test_runtime_integration.py
git commit -m "release: prepare 1.0.0-rc2"
~~~

### Task 4：本地全链路构建与候选证据

**文件：**

- 创建：G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\evidence\AE-RC2-READY-TO-REVIEW-20260823.md
- 修改：仅为本任务验证发现的问题所必需的文件

- [ ] **步骤 1：在任务临时根构建双平台 wheel**

Windows wheel 使用当前干净 worktree：

~~~text
$env:CODEX_TASK_TEMP='G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821'
$env:TEMP=$env:CODEX_TASK_TEMP
$env:TMP=$env:CODEX_TASK_TEMP
.venv\Scripts\maturin.exe build --release --locked --out G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\rc2-wheels\windows
~~~

Linux wheel 按已验证的 WSL 工具链，在同一任务根的 linux 子目录输出；若当前主机缺少可用 WSL/maturin，则记录为环境性部分验收，不能伪称双平台通过。

- [ ] **步骤 2：组装并验证 RC2 ZIP**

运行：

~~~text
$windowsWheel = (Get-ChildItem -LiteralPath G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\rc2-wheels\windows -Filter '*.whl' | Select-Object -First 1).FullName
$linuxWheel = (Get-ChildItem -LiteralPath G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\rc2-wheels\linux -Filter '*.whl' | Select-Object -First 1).FullName
if (-not $windowsWheel -or -not $linuxWheel) { throw 'fresh Windows and Linux wheels are required' }
.venv\Scripts\python.exe scripts/package_plugin.py --output G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\rc2-artifacts\astrbot_plugin_astrembodiment-1.0.0-rc2-win_linux_x86_64.zip --native-wheel $windowsWheel --native-wheel $linuxWheel
.venv\Scripts\python.exe scripts/validate_release.py --tag v1.0.0-rc2 --archive G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\rc2-artifacts\astrbot_plugin_astrembodiment-1.0.0-rc2-win_linux_x86_64.zip
Get-FileHash -Algorithm SHA256 G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\rc2-artifacts\astrbot_plugin_astrembodiment-1.0.0-rc2-win_linux_x86_64.zip
~~~

预期：归档包含一个 manifest 和两个内容寻址原生扩展，不包含 wheel、tests、crates、target 或虚拟环境。

- [ ] **步骤 3：运行最终质量门**

运行：

~~~text
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
.venv\Scripts\python.exe -m pytest -q -o cache_dir=G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\pytest-rc2-final
ruff format --check main.py astr_embodiment tests scripts
ruff check main.py astr_embodiment tests scripts
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo test --workspace --locked --offline
git diff --check
git status --short
~~~

预期：全部通过且只留下受控候选产物、证据与尚未提交的已知修复；任何失败必须在证据中标为 PASS、PARTIAL 或 NO-GO，而不是模糊成功。

- [ ] **步骤 4：写候选证据并更新 PR，不发布**

证据必须记录：commit、每项命令和退出码、Windows/Linux wheel 文件名与 SHA-256、ZIP SHA-256、版本三元组、当前 PR URL 与 CI 结果、已知限制。

若全部当前门禁为 PASS，创建本地注释标签 v1.0.0-rc2；不得推送 tag。用 gh pr edit 将既有 PR 标题更新为 release: prepare 1.0.0-rc2，并把测试证据写进 PR 正文。不得 merge、push tag、创建 GitHub Release 或调用 Marketplace。

- [ ] **步骤 5：记录验证修复的提交边界**

若最终验证暴露源码问题，先为该问题写对应 RED 测试，再只提交该任务已列出的确切文件；没有验证修复时不得创建空提交。证据始终保留在指定 G 盘任务根。
