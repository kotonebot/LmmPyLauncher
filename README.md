# LmmPyLauncher

通用 **Python 桌面应用 Windows 启动器**（Rust + Slint GUI）。面向以 pip 包分发的
Python 桌面应用：双击 exe 即可自动准备 Python 环境并启动应用，无需用户安装 Python。

## 功能

- **Splash 界面**：Slint 无边框窗口（深浅色跟随系统）、可选 logo、极细进度线 + 状态文字 + 可展开日志
- **首次启动**：询问用户所在地区（中国大陆 / 其他），决定 PyPI 与 Python 发行版下载源，持久化到 exe 旁的 `launcher.conf.json`
- **Python 自动部署**：本机无 Python 时自动下载 embed 版（国内走 TUNA 镜像，其他地区走 python.org），自举 pip / setuptools
- **pip 安装实时进度**：dry-run 解析包总数 → `Collecting` 行计数 → 状态栏分阶段文案
- **自动更新**：启动时对比本地与远端版本，自动安装新版本
- **主程序生命周期**：等待主程序出现首个可见窗口后隐藏启动器；主程序正常退出则启动器跟随退出；异常退出弹出崩溃报告窗口展示输出尾部 100 行
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
  "launch_command": "{python} -m {entry_point} {extra_args}",
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
| `uninstall_packages` | 安装前统一卸载的旧包名数组（历史包名迁移用） |
| `entry_point` | `python -m` 的模块路径 |
| `launch_command` | 启动命令模板，占位符 `{python}` / `{entry_point}` / `{extra_args}` |
| `icon` | exe 图标（.ico，可选） |
| `logoPath` | splash logo 图片（PNG/JPEG/SVG，可选；留空则不展示 logo） |

2. 构建：`LMM_LAUNCHER_JSON=<配置绝对路径> cargo build --release`
   （产物 `target/release/LmmPyLauncher.exe`）

   - `logoPath` / `icon` 相对**配置文件所在目录**解析，配置与资源文件放在一起即可
   - 不设置 `LMM_LAUNCHER_JSON` 时使用源码根的 `launcher.json`（缺失则自动从
     示例生成占位配置），便于直接开发调试
   - 集成方源码树保持干净：构建过程只读配置路径，不向启动器仓库写入任何文件
3. 分发：exe 与主程序包同目录放置，用户双击即用

## 运行时参数

- `--install-version=X`：安装指定版本（对 `install_packages` 全部生效，隐含 `--skip-update`）
- `--install-from-zip=<file>`：从 zip 安装补丁
- `--install-from-package=<file>`：从 .whl / .tar.gz 安装
- `--skip-update`：跳过更新检查直接启动
- 其余参数原样透传给主程序
- 运行时配置 `launcher.conf.json`（exe 旁）：`{"region": "cn" | "intl"}`

## License

MIT
