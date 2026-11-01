//! pip 调用封装：安装 / 卸载 / 查询版本。所有 pip 命令通过内嵌 Python 解释器执行，
//! 输出逐行回调，便于在 GUI 上实时显示。

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use serde_json::Value;

use crate::i18n::{fill, strings};
use crate::settings::Region;
use crate::version::Version;

/// 安装进度回调：(已完成包数, 总包数, 当前包名, 阶段)。
/// 阶段："download"（Collecting 下载中）、"install"（正在安装已下载的包）、"done"（全部完成）
pub type OnProgress = Box<dyn FnMut(usize, usize, &str, &str) + Send>;

/// 一次安装的进度统计状态，由 call_pip 的输出行解析更新
struct ProgressState {
    /// dry-run 解析出的待安装包总数
    total: usize,
    /// 已见过的包名（归一化后）
    done: std::collections::HashSet<String>,
}

/// Windows 下隐藏子进程的控制台窗口（GUI 模式启动控制台程序时会新开终端窗口）
pub trait HideConsoleWindow {
    fn hide_console_window(&mut self) -> &mut Self;
}

impl HideConsoleWindow for Command {
    #[cfg(windows)]
    fn hide_console_window(&mut self) -> &mut Self {
        // CREATE_NO_WINDOW
        self.creation_flags(0x0800_0000)
    }
    #[cfg(not(windows))]
    fn hide_console_window(&mut self) -> &mut Self {
        self
    }
}

/// 需要豁免 SSL 校验的镜像主机名（部分国内镜像证书链在老 Python 上验证失败）
pub const TRUSTED_HOSTS: &[&str] = &[
    "pypi.org",
    "files.pythonhosted.org",
    "pypi.python.org",
    "mirrors.aliyun.com",
    "mirrors.cloud.tencent.com",
    "mirrors.tuna.tsinghua.edu.cn",
];

/// 按优先级排列的 PyPI 镜像源（按用户选择的地区）
pub fn pip_servers() -> Vec<String> {
    match crate::settings::get() {
        Region::Cn => vec![
            "https://mirrors.tuna.tsinghua.edu.cn/pypi/web/simple".into(),
            "https://mirrors.aliyun.com/pypi/simple".into(),
            "https://mirrors.cloud.tencent.com/pypi/simple".into(),
            "https://pypi.org/simple".into(),
        ],
        Region::Intl => vec!["https://pypi.org/simple".into()],
    }
}

pub struct Pip {
    pub python: PathBuf,
    pub index_url: String,
    /// 输出行回调（stdout 与 stderr 混合，按行）
    pub on_line: Arc<Mutex<Box<dyn FnMut(&str) + Send>>>,
    /// 安装进度回调；None 时不统计（如升级 pip 本体）
    pub on_progress: Arc<Mutex<Option<OnProgress>>>,
    /// 阶段性状态文本回调（如"正在解析待安装包..."），供 GUI 状态栏展示
    pub on_status: Arc<Mutex<Option<Box<dyn FnMut(&str) + Send>>>>,
    /// 当前安装的进度状态（run_install 期间为 Some）
    progress: Arc<Mutex<Option<ProgressState>>>,
}

fn trusted_hosts_args() -> Vec<String> {
    let joined = TRUSTED_HOSTS.join(" ");
    vec!["--trusted-host".into(), joined]
}

impl Pip {
    pub fn new(python: PathBuf, index_url: &str) -> Self {
        Pip {
            python,
            index_url: index_url.to_string(),
            on_line: Arc::new(Mutex::new(Box::new(|_| {}))),
            on_progress: Arc::new(Mutex::new(None)),
            on_status: Arc::new(Mutex::new(None)),
            progress: Arc::new(Mutex::new(None)),
        }
    }

    /// 在启动器所在目录下查找内嵌 Python（路径来自编译期配置 launcher.json），
    /// 找不到时回退到 PATH 上的 python。
    pub fn find_python() -> Option<PathBuf> {
        let dist_dir = crate::app_config::get().python_dist_dir.clone();
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                if !dist_dir.is_empty() {
                    let embedded = dir.join(&dist_dir).join("python.exe");
                    if embedded.exists() {
                        return Some(embedded);
                    }
                    // 也尝试发行版根目录下的其它变体（保持兼容，扫描一层）
                    if let Some(root) = dist_dir.split(['/', '\\']).next() {
                        if let Ok(entries) = std::fs::read_dir(dir.join(root)) {
                            for entry in entries.flatten() {
                                let candidate = entry.path().join("python.exe");
                                if candidate.exists() {
                                    return Some(candidate);
                                }
                            }
                        }
                    }
                }
                // 自动部署的 embed Python（exe 旁 python-{version}/，见 deploy.rs）
                if let Some(p) = crate::deploy::installed_python_path() {
                    return Some(p);
                }
            }
        }
        which_python_on_path()
    }

    fn call_pip(&mut self, args: &[String]) -> std::io::Result<i32> {
        let mut cmd = Command::new(&self.python);
        cmd.args(["-m", "pip"])
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("PYTHONIOENCODING", "utf-8")
            .env("PYTHONUNBUFFERED", "1")
            .hide_console_window();

        let mut child = cmd.spawn()?;
        // 挂入启动器 Job Object：启动器退出时子进程一并被终止
        #[cfg(windows)]
        crate::job::assign_child(&child);
        // stdout 与 stderr 各起一个线程，逐行回调
        let mut handles = Vec::new();
        if let Some(out) = child.stdout.take() {
            handles.push(spawn_reader(
                out,
                Arc::clone(&self.on_line),
                Arc::clone(&self.progress),
                Arc::clone(&self.on_progress),
            ));
        }
        if let Some(err) = child.stderr.take() {
            handles.push(spawn_reader(
                err,
                Arc::clone(&self.on_line),
                Arc::clone(&self.progress),
                Arc::clone(&self.on_progress),
            ));
        }
        let status = child.wait()?;
        for h in handles {
            h.join().ok();
        }
        Ok(status.code().unwrap_or(-1))
    }

    /// dry-run 解析待安装包列表：`pip install --dry-run` 的 "Would install" 行
    /// 给出全部将安装的依赖，用于计算进度总数。
    fn would_install(&self, args: &[String]) -> Result<Vec<String>, String> {
        let mut dry: Vec<String> = args.to_vec();
        dry.push("--dry-run".into());
        dry.push("--progress-bar".into());
        dry.push("off".into());

        let mut cmd = Command::new(&self.python);
        cmd.args(["-m", "pip"])
            .args(&dry)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("PYTHONIOENCODING", "utf-8")
            .hide_console_window();
        let out = cmd
            .output()
            .map_err(|e| fill(strings().pip_start_fail, &[&e.to_string()]))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(fill(strings().analyze_fail, &[err.trim()]));
        }
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("Would install ") {
                // 条目形如 "PySide6-6.12.0"（空格分隔；旧版 pip 用 ", "，一并兼容），
                // 从最后一个 '-' 切掉版本号
                let pkgs = rest
                    .split(|c| c == ' ' || c == ',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|e| {
                        e.rsplit_once('-')
                            .map(|(name, _)| name.to_string())
                            .unwrap_or_else(|| e.to_string())
                    })
                    .collect();
                return Ok(pkgs);
            }
        }
        // "Would install" 缺失 = 无需安装任何包
        Ok(vec![])
    }

    /// 统一的带进度安装：先 dry-run 拿到包总数，再以 --progress-bar off 实际安装，
    /// 按 "Collecting <name>" 行计数回调进度。dry-run 失败不阻断安装（无进度而已）。
    fn run_install(&mut self, args: Vec<String>) -> Result<(), String> {
        self.notify_status(strings().analyzing_deps);
        let total = self.would_install(&args).map(|pkgs| pkgs.len()).unwrap_or_else(|e| {
            self.notify_line(&fill(strings().analyze_fail_log, &[&e]));
            0
        });
        // 无包可装时直接按原样安装（pip 会秒退并提示已满足）
        *self.progress.lock().unwrap() = Some(ProgressState {
            total,
            done: std::collections::HashSet::new(),
        });
        self.notify_status(strings().installing);
        let code = self.call_pip(&args).map_err(|e| fill(strings().pip_start_fail, &[&e.to_string()]));
        *self.progress.lock().unwrap() = None;
        let code = code?;
        if code != 0 {
            return Err(fill(strings().install_fail, &[&code.to_string()]));
        }
        Ok(())
    }

    fn notify_line(&self, line: &str) {
        if let Ok(mut f) = self.on_line.lock() {
            (f)(line);
        }
    }

    fn notify_status(&self, s: &str) {
        if let Ok(mut f) = self.on_status.lock() {
            if let Some(f) = f.as_mut() {
                (f)(s);
            }
        }
    }

    /// 升级 pip 本体
    pub fn upgrade_pip(&mut self) -> Result<(), String> {
        let mut args: Vec<String> = vec!["install".into(), "--upgrade".into(), "pip".into()];
        args.push("--index-url".into());
        args.push(self.index_url.clone());
        args.extend(trusted_hosts_args());
        let code = self
            .call_pip(&args)
            .map_err(|e| fill(strings().pip_start_fail, &[&e.to_string()]))?;
        if code != 0 {
            return Err(fill(strings().upgrade_fail, &[&code.to_string()]));
        }
        Ok(())
    }

    /// 卸载包（-y），失败时返回 Err
    pub fn uninstall(&mut self, package: &str) -> Result<(), String> {
        let args: Vec<String> = vec!["uninstall".into(), package.into(), "-y".into()];
        let code = self
            .call_pip(&args)
            .map_err(|e| fill(strings().pip_start_fail, &[&e.to_string()]))?;
        if code != 0 {
            return Err(fill(strings().uninstall_fail, &[package, &code.to_string()]));
        }
        Ok(())
    }

    pub fn install_version(&mut self, package: &str, version: &str) -> Result<(), String> {
        let mut args: Vec<String> = vec![
            "install".into(),
            format!("{package}=={version}"),
            "--progress-bar".into(),
            "off".into(),
            "--index-url".into(),
            self.index_url.clone(),
        ];
        args.extend(trusted_hosts_args());
        self.run_install(args)
    }

    /// 从 .whl / .tar.gz 文件安装
    pub fn install_from_file(&mut self, path: &Path) -> Result<(), String> {
        let mut args: Vec<String> = vec![
            "install".into(),
            path.to_string_lossy().into_owned(),
            "--upgrade".into(),
            "--progress-bar".into(),
            "off".into(),
            "--index-url".into(),
            self.index_url.clone(),
        ];
        args.extend(trusted_hosts_args());
        self.run_install(args)
    }

    /// 从文件夹安装（zip 解压后，--no-index --find-links），packages 为要装的包名
    pub fn install_from_folder(&mut self, folder: &Path, packages: &[String]) -> Result<(), String> {
        let mut args: Vec<String> = vec![
            "install".into(),
            "--find-links".into(),
            folder.to_string_lossy().into_owned(),
        ];
        args.extend(packages.iter().cloned());
        args.extend([
            "--no-index".into(),
            "--upgrade".into(),
            "--progress-bar".into(),
            "off".into(),
        ]);
        self.run_install(args)
    }

    /// 查询本地已安装版本（`pip show`）
    pub fn local_version(&mut self, package: &str) -> Option<Version> {
        let mut cmd = Command::new(&self.python);
        cmd.args(["-m", "pip", "show", package])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .hide_console_window();
        let out = cmd.output().ok()?;
        if !out.status.success() {
            return None;
        }
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if let Some(v) = line.strip_prefix("Version: ") {
                return Some(Version::new(v.trim()));
            }
        }
        None
    }

    /// 列出远端所有版本（`pip index versions --json`），升序返回
    pub fn list_versions(&mut self, package: &str, pre: bool) -> Result<Vec<Version>, String> {
        let mut args: Vec<String> = vec![
            "index".into(),
            "versions".into(),
            package.into(),
            "--json".into(),
        ];
        if pre {
            args.push("--pre".into());
        }
        args.push("--index-url".into());
        args.push(self.index_url.clone());
        args.extend(trusted_hosts_args());

        let mut cmd = Command::new(&self.python);
        cmd.args(["-m", "pip"])
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("PYTHONIOENCODING", "utf-8")
            .hide_console_window();
        let out = cmd
            .output()
            .map_err(|e| fill(strings().pip_start_fail, &[&e.to_string()]))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(fill(strings().versions_fail, &[err.trim()]));
        }
        let text = String::from_utf8_lossy(&out.stdout);
        // pip 输出中可能混有非 JSON 行，取第一行以 '{' 开头的内容
        let json_text = text
            .lines()
            .skip_while(|l| !l.trim_start().starts_with('{'))
            .collect::<Vec<_>>()
            .join("\n");
        let v: Value = serde_json::from_str(&json_text)
            .map_err(|e| fill(strings().parse_versions_fail, &[&e.to_string()]))?;
        let mut versions: Vec<Version> = v["versions"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str())
                    .map(Version::new)
                    .collect()
            })
            .unwrap_or_default();
        versions.sort();
        Ok(versions)
    }
}

/// 通过 `where python` 查找 PATH 上的 Python
fn which_python_on_path() -> Option<PathBuf> {
    Command::new("where")
        .arg("python")
        .creation_flags(0x0800_0000)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .and_then(|s| s.lines().next().map(|l| PathBuf::from(l.trim())))
}

type OnLine = Box<dyn FnMut(&str) + Send>;

fn spawn_reader<R: std::io::Read + Send + 'static>(
    reader: R,
    on_line: Arc<Mutex<OnLine>>,
    progress: Arc<Mutex<Option<ProgressState>>>,
    on_progress: Arc<Mutex<Option<OnProgress>>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let br = BufReader::new(reader);
        for line in br.lines() {
            match line {
                Ok(l) => {
                    if !l.trim().is_empty() {
                        track_progress(&progress, &on_progress, &l);
                        if let Ok(mut f) = on_line.lock() {
                            (f)(&l);
                        }
                    }
                }
                Err(_) => break,
            }
        }
    })
}

/// 按 pip 的 --progress-bar off 输出行更新安装进度：
/// "Collecting <name>" 计一个包；"Successfully installed ..." 收尾补满。
fn track_progress(
    progress: &Arc<Mutex<Option<ProgressState>>>,
    on_progress: &Arc<Mutex<Option<OnProgress>>>,
    line: &str,
) {
    let mut guard = match progress.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let Some(state) = guard.as_mut() else { return };
    if state.total == 0 {
        return;
    }

    let current;
    let mut phase = "download";
    if let Some(name) = line.strip_prefix("Collecting ") {
        // 去掉可能的上级依赖标注（"X (from Y)"），只留包名本身
        let name = name.split(" (from").next().unwrap_or(name).trim();
        // 归一化（PEP 503）：忽略大小写与 -/_ 差异后去重计数
        let norm = normalize_pkg(name);
        if state.done.insert(norm) {
            current = name.to_string();
        } else {
            return;
        }
    } else if line.starts_with("Installing collected packages: ") {
        // 下载全部完成，进入本地安装阶段
        phase = "install";
        current = String::new();
    } else if line.starts_with("Successfully installed ") {
        // 收尾：实际安装的包可能比 dry-run 多，安装成功即视为全部完成
        while state.done.len() < state.total {
            state.done.insert(format!("\0final{}", state.done.len()));
        }
        phase = "done";
        current = String::new();
    } else {
        return;
    }

    let done = state.done.len().min(state.total);
    let total = state.total;
    drop(guard);
    if let Ok(mut cb) = on_progress.lock() {
        if let Some(f) = cb.as_mut() {
            (f)(done, total, &current, phase);
        }
    }
}

/// PEP 503 名称归一化：小写、_ 归一为 -、去掉 extras（name[extra]）
fn normalize_pkg(name: &str) -> String {
    let base = name.split('[').next().unwrap_or(name);
    base.trim().to_lowercase().replace('_', "-")
}
