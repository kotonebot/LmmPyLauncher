# LmmPyLauncher

通用 **Python 应用 Windows 启动器**（Rust；GUI 版用 Slint）。面向以 pip 包分发的
Python 应用：双击 exe 即可自动准备 Python 环境并启动应用，无需用户安装 Python。

## 形态

同一份源码树产出两个二进制，各自定位明确、互不干扰：

| 产物 | 子系统 | 用途 |
|---|---|---|
| `LmmPyLauncher.exe` | windows | **GUI 版**：双击启动，Slint splash，负责 Python 部署、包安装/更新，等待主程序窗口出现后自动隐藏 |
| `LmmPyLauncherCli.exe` | console | **CLI 版**：无窗口，不安装不更新，只检查环境后直接启动主程序；参数与退出码透传 |

两者放在同一目录时共享 `python-{ver}/`、`conf/`、`logs/`。**首次使用必须先跑一次 GUI 版**
完成环境准备；CLI 版检测到环境未就绪会直接报错并提示这一点，不会自行安装。

两者的安装、更新、Python 部署逻辑来自同一个库（`src/lib.rs`，不依赖 Slint），
因此 CLI 版体积约 1.8 MB，GUI 版约 17 MB。

## 功能

- **Splash 界面**：Slint 无边框窗口（深浅色跟随系统）、可选 logo、极细进度线 + 状态文字 + 可展开日志
- **首次启动**：询问用户所在地区（中国大陆 / 其他），决定 PyPI 与 Python 发行版下载源，持久化到 exe 旁的 `launcher.conf.json`（GUI 弹模态窗口，CLI 走 stdin）
- **Python 自动部署**：本机无 Python 时自动下载 embed 版（国内走 TUNA 镜像，其他地区走 python.org），自举 pip / setuptools
- **pip 安装实时进度**：dry-run 解析包总数 → `Collecting` 行计数 → 状态栏分阶段文案
- **自动更新**：启动时对比本地与远端版本，自动安装新版本
- **主程序生命周期（GUI）**：等待主程序出现首个可见窗口后隐藏启动器；正常退出则跟随退出；异常退出弹出崩溃报告窗口展示输出尾部 100 行
- **主程序生命周期（CLI）**：子进程继承当前控制台；退出码原样作为本进程退出码；Ctrl+C 交给子进程处理，启动器继续等待并透传结果
- **CLI 零前置动作**：不探测镜像、不检查更新、不安装、不部署 Python，启动耗时约 0.5 秒；环境未就绪时提示"请先运行 GUI 版"
- **进程清理**：Windows Job Object 保证启动器被强杀时子进程一并终止
- **i18n**：全部界面与流程文案跟随系统语言（简体中文 / English）

## 使用（集成方）

1. 准备你的配置目录（建议与 logo/icon 资源放在一起），以 `launcher.example.json`
   为模板填写。配置在编译期嵌入二进制：

```json
{
  "app_name": "你的应用名",
  "python_version": "3.10.11",
  "python_dist_dir": "",
  "install_packages": ["your-app"],
  "uninstall_packages": [],
  "entry_point": "your_app.main",
  "launch_command": "{python} -m {entry_point} gui",
  "launch_command_cli": "{python} -m {entry_point} {extra_args}",
  "icon": "path/to/icon.ico",
  "logoPath": "logo.png"
}
```

| 字段 | 说明 |
|---|---|
| `app_name` | 应用名（窗口标题 / 界面标题） |
| `python_version` | 自动部署的 embed Python 版本 |
| `python_dist_dir` | 内嵌 Python 发行版目录（相对 exe，可空） |
| `install_packages` | 要安装/更新的 pip 包名数组（纯名称，无版本约束） |
| `uninstall_packages` | 安装前统一卸载的旧包名数组（历史包名迁移用，可为空数组） |
| `entry_point` | `python -m` 的模块路径 |
| `launch_command` | **GUI 版**启动命令模板，占位符 `{python}` / `{entry_point}`；通常不含 `{extra_args}`（GUI exe 不接受参数） |
| `launch_command_cli` | **CLI 版**启动命令模板，可含 `{extra_args}`；缺省时回退 `launch_command`（此时参数被丢弃） |
| `icon` | exe 图标（.ico，可选） |
| `logoPath` | splash logo 图片（PNG/JPEG/SVG，可选；留空则不展示 logo） |

2. 构建：`LMM_LAUNCHER_JSON=<配置绝对路径> cargo build --release --bins`
   产物为 `target/release/LmmPyLauncher.exe` 与 `target/release/LmmPyLauncherCli.exe`。

   - `logoPath` / `icon` 相对**配置文件所在目录**解析，配置与资源文件放在一起即可
   - 不设置 `LMM_LAUNCHER_JSON` 时使用源码根的 `launcher.json`（缺失则自动从
     示例生成占位配置），便于直接开发调试
   - 集成方源码树保持干净：构建过程只读配置路径，不向启动器仓库写入任何文件
   - 只需一种形态时可 `--bin LmmPyLauncher` 或 `--bin LmmPyLauncherCli` 单独构建
3. 分发：exe 与主程序包同目录放置，用户双击即用；两种形态按需一起分发

## 运行时参数

两种形态的参数策略完全不同：

### CLI 版：全量透传

`LmmPyLauncherCli.exe <任意参数>` 不做任何解释——**所有**命令行内容原样交给主程序，
包括 `--install-*` 这类开关（CLI 不安装不更新，这些开关对它没有意义，静默吞掉反而
更容易误判）。主程序退出码即本进程退出码，可直接用于脚本判断。

```bash
LmmPyLauncherCli.exe list tasks
LmmPyLauncherCli.exe invoke auto_live --count 10
LmmPyLauncherCli.exe --debug run > out.txt
```

环境未就绪时（找不到解释器 / 包未安装）直接报错退出（码 1），提示先运行 GUI 版。

### GUI 版：拒绝参数，只认启动器开关

- `--install-version=X`：安装指定版本（对 `install_packages` 全部生效，隐含 `--skip-update`）
- `--install-from-zip=<file>`：从 zip 安装补丁
- `--install-from-package=<file>`：从 .whl / .tar.gz 安装
- `--skip-update`：跳过更新检查直接启动

其余参数一律视为错误：未知开关被丢弃，位置参数直接报错退出（`unsupported file type`），
避免"传了参数但没生效"的静默失败。

### 通用

- 运行时配置 `launcher.conf.json`（exe 旁）：`{"region": "cn" | "intl"}`，仅 GUI 版会写入/询问

## 架构

```
src/
  lib.rs        共享逻辑：参数解析、安装/更新、子进程生命周期、退出码
  host.rs       交互抽象（询问地区 / 部署确认 / 崩溃报告 / 退出主循环）
  app_config.rs 编译期配置（launcher.json）
  deploy.rs     embed Python 部署
  pip.rs        pip 调用与进度解析
  settings.rs   运行时配置 launcher.conf.json
  win32.rs      Win32 辅助（无边框、消息框、模态对话框）
  bin/gui.rs    GUI 版入口（windows 子系统，Slint）
  bin/cli.rs    CLI 版入口（console 子系统，stderr）
```

`lib` 不认识任何 UI：需要与用户交互时通过 `host::Host` 调用，由各 bin 注册实现。
因此新增产物（例如托盘版）只需再写一个 bin，不必改安装逻辑。

## License

MIT
