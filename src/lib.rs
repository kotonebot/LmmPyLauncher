//! LmmPyLauncher — 通用 Python 桌面应用启动器：公共逻辑库。
//!
//! 本 crate 不依赖 Slint，由两个二进制复用：
//! - `LmmPyLauncher`（`src/bin/gui.rs`，windows 子系统，Slint splash）
//! - `LmmPyLauncherCli`（`src/bin/cli.rs`，console 子系统，stderr 输出）
//!
//! 职责：
//! 1. 首次启动询问地区（决定 PyPI / Python 发行版下载源）
//! 2. 寻找最快的 PyPI 镜像源
//! 3. 安装与更新 pip 和主程序（包名来自编译期配置 launcher.json）
//! 4. 启动主程序
//! 5. 恢复模式（手动选择版本安装，流程保留）
//!
//! 需要与用户交互的地方（询问地区、部署确认、崩溃报告、退出主循环）都通过
//! [`host::Host`] 抽象，由各 bin 在启动时注册自己的实现。

pub mod app_config;
pub mod deploy;
pub mod host;
pub mod i18n;
#[cfg(windows)]
pub mod job;
pub mod logging;
pub mod pip;
pub mod settings;
#[cfg(windows)]
pub mod win32;
pub mod version;

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::Stdio;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use crate::pip::{HideConsoleWindow, Pip};
use crate::settings::Region;

pub const BOOTSTRAP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 安装/升级进行中标志：WM_CLOSE 的退出确认需要跨模块读取（win32::wnd_proc）
pub static BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 运行模式。编译期由 bin 决定，影响子进程 stdio、参数透传与退出方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// GUI：子进程输出走管道（供崩溃报告展示），等待首个可见窗口后隐藏启动器，
    /// 不接受额外命令行参数。
    Gui,
    /// CLI：子进程输出直连控制台，不等待窗口，退出码透传，全部参数转发给主程序。
    Cli,
}

/// UI 事件（工作线程 → 消费端）
pub enum UiMsg {
    /// 追加一行日志
    Log(String),
    /// 更新状态栏文案
    Status(String),
    /// 更新进度（0-100）
    Progress(f32),
    /// 忙碌状态（GUI 用于禁用关闭按钮）
    Busy(bool),
    /// 恢复模式 UI 已移除，但恢复流程代码保留，消息通道维持不变
    #[allow(dead_code)]
    Versions(Vec<String>),
}

/// 工作线程与 UI 之间的单向通道。
///
/// 这里只持有发送端：GUI bin 把消息渲染到 Slint，CLI bin 直接打印到 stderr，
/// 因此 lib 侧不需要依赖任何 UI 框架。
pub struct Ui {
    tx: Sender<UiMsg>,
}

impl Ui {
    /// 创建通道，返回 `(Ui, 接收端)`。接收端由 bin 起线程消费。
    pub fn new() -> (Arc<Ui>, Receiver<UiMsg>) {
        let (tx, rx) = channel::<UiMsg>();
        (Arc::new(Ui { tx }), rx)
    }

    /// 追加一行日志
    pub fn log(&self, line: &str) {
        let _ = self.tx.send(UiMsg::Log(line.to_string()));
    }

    /// 更新状态文案
    pub fn status(&self, s: &str) {
        let _ = self.tx.send(UiMsg::Status(s.to_string()));
    }

    /// 更新进度（0-100）
    pub fn progress(&self, p: f32) {
        let _ = self.tx.send(UiMsg::Progress(p));
    }

    /// 更新忙碌状态
    pub fn busy(&self, b: bool) {
        let _ = self.tx.send(UiMsg::Busy(b));
    }

    /// 恢复模式 UI 已移除，恢复流程保留以备后用
    #[allow(dead_code)]
    pub fn versions(&self, v: Vec<String>) {
        let _ = self.tx.send(UiMsg::Versions(v));
    }
}

/// 命令行参数
/// 命令行参数。
pub struct Args {
    /// 指定安装版本
    pub install_version: Option<String>,
    /// 从 zip 安装
    pub install_from_zip: Option<std::path::PathBuf>,
    /// 从 .whl / .tar.gz 安装
    pub install_from_package: Option<std::path::PathBuf>,
    /// 跳过更新检查
    pub skip_update: bool,
    /// 透传给主程序（仅 CLI 模式生效）
    pub extra_args: Vec<String>,
}

/// 解析启动器自身参数。
///
/// 启动器独占的开关（`--install-*`、`--skip-update`）一律由启动器消费；
/// `.zip` / `.whl` / `.tar.gz` 位置参数视为本地安装包。其余参数的处理取决于模式：
/// GUI 模式拒绝（它是纯 GUI 入口，参数无从去处），CLI 模式原样透传给主程序。
///
/// :param mode: 运行模式。
/// :return: 解析结果。
/// :raises String: GUI 模式下出现非法位置参数时。
pub fn parse_args(mode: Mode) -> Result<Args, String> {
    let s = i18n::strings();
    let mut args = Args {
        install_version: None,
        install_from_zip: None,
        install_from_package: None,
        skip_update: false,
        extra_args: Vec::new(),
    };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        let a = &argv[i];
        if let Some(v) = a.strip_prefix("--install-version=") {
            args.install_version = Some(v.to_string());
        } else if let Some(v) = a.strip_prefix("--install-from-zip=") {
            args.install_from_zip = Some(std::path::PathBuf::from(v));
        } else if let Some(v) = a.strip_prefix("--install-from-package=") {
            args.install_from_package = Some(std::path::PathBuf::from(v));
        } else if a == "--install-version" {
            i += 1;
            args.install_version = argv.get(i).cloned();
        } else if a == "--install-from-zip" {
            i += 1;
            args.install_from_zip = argv.get(i).map(std::path::PathBuf::from);
        } else if a == "--install-from-package" {
            i += 1;
            args.install_from_package = argv.get(i).map(std::path::PathBuf::from);
        } else if a == "--skip-update" {
            args.skip_update = true;
        } else {
            // 其余参数：先判断是否本地安装包，再决定透传还是报错
            let lower = a.to_ascii_lowercase();
            let is_zip = lower.ends_with(".zip");
            let is_pkg = lower.ends_with(".whl") || lower.ends_with(".tar.gz");
            if is_zip {
                if args.install_from_zip.is_none() {
                    args.install_from_zip = Some(std::path::PathBuf::from(a));
                }
            } else if is_pkg {
                if args.install_from_package.is_none() {
                    args.install_from_package = Some(std::path::PathBuf::from(a));
                }
            } else if mode == Mode::Cli {
                // CLI 模式：未知开关与位置参数全部转发给主程序
                args.extra_args.push(a.clone());
            } else {
                // GUI 模式：exe 不接受参数，明确报错而不是静默丢弃
                return Err(i18n::fill(s.unsupported_file_type, &[a]));
            }
        }
        i += 1;
    }
    Ok(args)
}

/// 测试 URL 是否可访问（HEAD 请求返回 2xx）
fn test_url_availability(url: &str) -> bool {
    ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .head(url)
        .call()
        .map(|_| true)
        .or_else(|e| match e {
            // 4xx 也说明服务器可达
            ureq::Error::Status(_, _) => Ok(true),
            other => Err(other),
        })
        .unwrap_or(false)
}

pub(crate) fn get_working_pip_server(ui: &Ui) -> Option<String> {
    let s = i18n::strings();
    for server in pip::pip_servers() {
        ui.log(&i18n::fill(s.testing_server, &[&server]));
        if test_url_availability(&server) {
            ui.log(&i18n::fill(s.server_ok, &[&server]));
            return Some(server);
        }
    }
    ui.log(s.no_pip_server);
    None
}

/// 接上 pip 安装进度回调：下载阶段显示"正在下载 <包名> (done/total)"，
/// 本地安装阶段显示"正在安装共 X 个包"，完成后显示"安装完成"；
/// 进度条把 done/total 映射到 10–60% 区间
fn attach_progress(pip: &mut Pip, ui: &Arc<Ui>) {
    let ui2 = Arc::clone(ui);
    pip.on_progress = Arc::new(Mutex::new(Some(Box::new(move |done, total, name, phase| {
        let s = i18n::strings();
        let frac = if total > 0 { done as f32 / total as f32 } else { 0.0 };
        ui2.progress(10.0 + 50.0 * frac);
        match phase {
            "install" => ui2.status(&i18n::fill(s.installing_pkgs, &[&done.to_string()])),
            "done" => ui2.status(s.install_done),
            _ => ui2.status(&i18n::fill(s.downloading_pkg, &[name, &done.to_string(), &total.to_string()])),
        }
    }))));
    let ui2 = Arc::clone(ui);
    pip.on_status = Arc::new(Mutex::new(Some(Box::new(move |s| ui2.status(s)))));
}

/// 安装前统一卸载 uninstall_packages 中的旧包。失败时记录日志并继续。
fn clean(pip: &mut Pip, ui: &Ui) {
    let s = i18n::strings();
    ui.log(s.uninstalling_existing);
    for pkg in &app_config::get().uninstall_packages {
        if let Err(e) = pip.uninstall(pkg) {
            ui.log(&i18n::fill(s.uninstall_fail_continue, &[pkg, &e]));
        }
    }
}

fn install_packages_version(pip: &mut Pip, ui: &Ui, version: &str) -> Result<(), String> {
    let s = i18n::strings();
    clean(pip, ui);
    for pkg in &app_config::get().install_packages {
        ui.log(&i18n::fill(s.installing_version_log, &[pkg, version]));
        pip.install_version(pkg, version)?;
    }
    Ok(())
}

fn install_from_zip(pip: &mut Pip, ui: &Ui, zip_path: &Path) -> Result<(), String> {
    let s = i18n::strings();
    if !zip_path.exists() {
        return Err(i18n::fill(s.zip_not_found, &[&zip_path.display().to_string()]));
    }
    ui.log(&i18n::fill(s.install_from_zip, &[&zip_path.display().to_string()]));
    let temp_dir = tempfile::tempdir().map_err(|e| i18n::fill(s.temp_dir_fail, &[&e.to_string()]))?;
    ui.log(s.extracting_zip);
    let file = std::fs::File::open(zip_path).map_err(|e| i18n::fill(s.open_zip_fail, &[&e.to_string()]))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| i18n::fill(s.invalid_zip, &[&e.to_string()]))?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| i18n::fill(s.read_zip_fail, &[&e.to_string()]))?;
        let Some(name) = entry.enclosed_name() else { continue };
        let out_path = temp_dir.path().join(name);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut out = std::fs::File::create(&out_path).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        }
    }
    clean(pip, ui);
    let pkgs = app_config::get().install_packages.clone();
    for pkg in &pkgs {
        ui.log(&i18n::fill(s.installing_pkg_log, &[pkg]));
    }
    pip.install_from_folder(temp_dir.path(), &pkgs)
}

fn install_from_package(pip: &mut Pip, ui: &Ui, path: &Path) -> Result<(), String> {
    let s = i18n::strings();
    if !path.exists() {
        return Err(i18n::fill(s.pkg_not_found, &[&path.display().to_string()]));
    }
    let lower = path.to_string_lossy().to_ascii_lowercase();
    if !(lower.ends_with(".whl") || lower.ends_with(".tar.gz")) {
        return Err(i18n::fill(s.not_pkg_file, &[&path.display().to_string()]));
    }
    clean(pip, ui);
    ui.log(&i18n::fill(s.install_from_pkg, &[&path.display().to_string()]));
    pip.install_from_file(path)
}

/// 默认流程：更新 pip、逐个检查/安装 install_packages 中的包。
fn install_pip_and_packages(pip: &mut Pip, ui: &Ui) -> Result<(), String> {
    let s = i18n::strings();
    ui.log(s.upgrading_pip);
    pip.upgrade_pip()?;

    let app = &app_config::get().app_name;
    ui.log(s.checking_update);
    for pkg in &app_config::get().install_packages {
        let latest = pip
            .list_versions(pkg, false)?
            .into_iter()
            .next_back()
            .ok_or_else(|| i18n::fill(s.no_versions, &[pkg]))?;
        let local = pip.local_version(pkg);

        match local {
            None => {
                ui.log(s.not_installed);
                clean(pip, ui);
                pip.install_version(pkg, &latest.raw)?;
            }
            Some(local) if local < latest => {
                ui.log(&i18n::fill(s.new_version, &[&local.raw, &latest.raw]));
                clean(pip, ui);
                pip.install_version(pkg, &latest.raw)?;
            }
            Some(local) => {
                ui.log(&i18n::fill(s.up_to_date, &[app, &local.raw]));
            }
        }
    }
    Ok(())
}

/// 主程序退出结果。
pub struct RunOutcome {
    /// 主程序退出码（无法获取时为 -1）
    pub code: i32,
    /// 输出尾部（stdout+stderr 最近 100 行）。仅 GUI 模式收集，CLI 模式为空。
    pub tail: String,
}

/// 运行主程序并阻塞等待其退出。
///
/// GUI 模式：子进程输出走管道（仅供崩溃报告展示），等待首个可见窗口后隐藏启动器，
/// 不接受额外命令行参数；CLI 模式：子进程继承当前控制台，不等待窗口，参数全部转发。
///
/// :param ui: 事件通道，用于上报状态与日志。
/// :param extra_args: 透传给主程序的参数（GUI 模式忽略）。
/// :param mode: 运行模式。
/// :return: 子进程退出码与输出尾部。
/// :raises String: 解释器缺失、命令模板为空或进程无法启动时。
pub fn run_app(ui: &Ui, extra_args: &[String], mode: Mode) -> Result<RunOutcome, String> {
    let s = i18n::strings();
    let config = app_config::get();
    let python = Pip::find_python().ok_or(s.python_not_found)?;

    ui.status(&i18n::fill(s.waiting_start, &[&config.app_name]));

    // 命令模板：CLI 版优先用 launch_command_cli（含 {extra_args} 占位），
    // 缺省时退回 launch_command。GUI 模板里没有占位符，参数自然被丢弃。
    let template = match mode {
        Mode::Cli => config.launch_command_cli.as_deref().unwrap_or(&config.launch_command),
        Mode::Gui => config.launch_command.as_str(),
    };
    // GUI 版 exe 定位为纯 GUI 入口：任何外部参数都不下传
    let extra: &[String] = match mode {
        Mode::Gui => &[],
        Mode::Cli => extra_args,
    };
    let mut parts: Vec<String> = Vec::new();
    for tok in template.split_whitespace() {
        match tok {
            "{python}" => parts.push(python.to_string_lossy().into_owned()),
            "{extra_args}" => parts.extend(extra.iter().cloned()),
            other => parts.push(other.replace("{entry_point}", &config.entry_point)),
        }
    }
    ui.log(&i18n::fill(s.running_cmd, &[&parts.join(" ")]));

    let (program, cmd_args) = parts
        .split_first()
        .ok_or(s.launch_cmd_empty)?;
    let mut cmd = std::process::Command::new(program);
    cmd.args(cmd_args)
        .env("no_proxy", "localhost, 127.0.0.1, ::1")
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUNBUFFERED", "1");
    match mode {
        // GUI：输出被管道截获，只用于崩溃时展示尾部；不给子进程弹控制台
        Mode::Gui => {
            cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).hide_console_window();
        }
        // CLI：直接继承启动器所在的控制台，用户能看到输出、也能交互
        Mode::Cli => {
            cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
        }
    }
    let mut child = cmd.spawn().map_err(|e| i18n::fill(s.spawn_fail, &[&e.to_string()]))?;
    // 挂入 Job Object：启动器退出时主程序一并被终止
    job::assign_child(&child);

    // GUI：等待主程序出现第一个可见顶层窗口后再隐藏启动器（最多 30s）。
    // 若在窗口出现前就退出（启动即崩），不隐藏，直接走崩溃报告流程。
    // CLI：子进程是控制台程序，没有窗口，跳过这段等待。
    if mode == Mode::Gui {
        #[cfg(windows)]
        {
            let pid = child.id();
            let mut appeared = false;
            for _ in 0..300 {
                if child_has_visible_window(pid) {
                    appeared = true;
                    break;
                }
                if child.try_wait().map_or(false, |o| o.is_some()) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            if appeared {
                // 主程序窗口已接管前台：隐藏启动器窗口
                hide_launcher_window();
            }
        }
    }

    // 收集主程序输出尾部（stdout+stderr 合计最近 100 行），供崩溃提示窗口展示。
    // CLI 模式下句柄为 inherit，take() 得到 None，不会收集。
    let tail: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut handles = Vec::new();
    if let Some(out) = child.stdout.take() {
        handles.push(spawn_tail_reader(out, Arc::clone(&tail)));
    }
    if let Some(err) = child.stderr.take() {
        handles.push(spawn_tail_reader(err, Arc::clone(&tail)));
    }
    let output = child.wait().map_err(|e| i18n::fill(s.wait_fail, &[&e.to_string()]))?;
    for h in handles {
        h.join().ok();
    }

    let code = output.code().unwrap_or(-1);
    let tail_text = tail.lock().map(|t| t.join("\n")).unwrap_or_default();
    if code == 0 {
        ui.progress(100.0);
        ui.status(&i18n::fill(s.app_exited, &[&config.app_name]));
        ui.log(&i18n::fill(s.app_exited, &[&config.app_name]));
    } else {
        ui.log(&i18n::fill(s.crash_exit_log, &[&code.to_string()]));
        logging::error(&i18n::fill(s.crash_exit_log, &[&code.to_string()]));
    }
    Ok(RunOutcome { code, tail: tail_text })
}


/// 隐藏启动器主窗口（主程序运行期间不再需要它）。
///
/// 只对 GUI 版有意义：CLI 版进程没有窗口，`hwnd_of` 返回 None，调用为空操作。
#[cfg(windows)]
pub fn hide_launcher_window() {
    #[link(name = "user32")]
    extern "system" {
        fn ShowWindow(hwnd: *mut core::ffi::c_void, n_cmd_show: i32) -> i32;
    }
    const SW_HIDE: i32 = 0;
    if let Some(hwnd) = hwnd_of() {
        unsafe {
            ShowWindow(hwnd, SW_HIDE);
        }
    }
}

#[cfg(not(windows))]
pub fn hide_launcher_window() {}

/// 检查指定 PID 的进程是否已有可见的顶层窗口
#[cfg(windows)]
fn child_has_visible_window(pid: u32) -> bool {
    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(callback: extern "system" fn(*mut core::ffi::c_void, isize) -> i32, lparam: isize) -> i32;
        fn GetWindowThreadProcessId(hwnd: *mut core::ffi::c_void, pid: *mut u32) -> u32;
        fn IsWindowVisible(hwnd: *mut core::ffi::c_void) -> i32;
    }
    let mut state = (pid, false);
    extern "system" fn enum_proc(hwnd: *mut core::ffi::c_void, lparam: isize) -> i32 {
        let state = unsafe { &mut *(lparam as *mut (u32, bool)) };
        unsafe {
            let mut wpid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut wpid);
            if wpid == state.0 && IsWindowVisible(hwnd) != 0 {
                state.1 = true;
                return 0;
            }
        }
        1
    }
    unsafe { EnumWindows(enum_proc, &mut state as *mut _ as isize); }
    state.1
}

/// 读取主程序子进程的一路输出，保留最近 100 行到 tail
fn spawn_tail_reader<R: std::io::Read + Send + 'static>(
    reader: R,
    tail: Arc<Mutex<Vec<String>>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let br = BufReader::new(reader);
        for line in br.lines().flatten() {
            let mut t = match tail.lock() {
                Ok(t) => t,
                Err(_) => return,
            };
            if t.len() >= 100 {
                t.remove(0);
            }
            t.push(line);
        }
    })
}

/// 主流程（在后台线程执行）：询问地区 → 定位/部署 Python → 安装或更新 → 启动主程序。
///
/// :param ui: 事件通道。
/// :param args: 已解析的命令行参数。
/// :param mode: 运行模式，决定参数透传与子进程 stdio。
/// :return: 主程序退出结果（安装阶段失败则返回 Err）。
/// :raises String: 任一安装/更新/启动阶段失败时。
pub fn run_main(ui: Arc<Ui>, args: Args, mode: Mode) -> Result<RunOutcome, String> {
    let s = i18n::strings();
    ui.busy(true);
    ui.progress(0.0);
    ui.status(s.initializing);

    let result = (|| -> Result<RunOutcome, String> {
        // 首次启动：询问地区并持久化（决定 PyPI / Python 发行版下载源）
        let _region: Region = settings::ensure();

        let python = match Pip::find_python() {
            Some(p) => p,
            // 本机没有任何可用的 Python：询问用户后自动下载部署 embed 版
            None => {
                ui.status(s.python_not_found);
                deploy::ensure_python(&ui)?
            }
        };
        logging::info(&i18n::fill(s.python_path_log, &[&python.display().to_string()]));

        ui.status(s.finding_mirror);
        let server = get_working_pip_server(&ui).ok_or(s.no_pip_server_err)?;

        let mut pip = Pip::new(python, &server);
        {
            let ui2 = Arc::clone(&ui);
            pip.on_line = Arc::new(Mutex::new(Box::new(move |line| ui2.log(line))));
        }
        attach_progress(&mut pip, &ui);

        ui.progress(10.0);
        if let Some(zip) = &args.install_from_zip {
            ui.status(s.installing_zip_status);
            install_from_zip(&mut pip, &ui, zip)?;
        } else if let Some(pkg) = &args.install_from_package {
            ui.status(s.installing_pkg_status);
            install_from_package(&mut pip, &ui, pkg)?;
        } else if let Some(v) = &args.install_version {
            ui.status(s.installing_version_status);
            install_packages_version(&mut pip, &ui, v)?;
        } else if !args.skip_update {
            ui.status(s.install_update_status);
            install_pip_and_packages(&mut pip, &ui)?;
        }

        ui.progress(60.0);
        run_app(&ui, &args.extra_args, mode)
    })();

    if let Err(e) = &result {
        ui.status(&i18n::fill(s.fatal_error, &[e]));
        ui.log(&i18n::fill(s.fatal_error, &[e]));
        // 反馈指引随形态不同：GUI 让用户附窗口截图，CLI 只能附终端输出
        ui.log(&host::get().feedback_hint(s.feedback_hint));
        logging::error(e);
    }
    ui.busy(false);
    result
}

/// 恢复模式：列出可用版本供用户选择
/// 恢复模式流程（UI 入口已移除，流程代码保留以备后用）
#[allow(dead_code)]
fn run_recovery(ui: Arc<Ui>, server: Option<String>) {
    let s = i18n::strings();
    ui.busy(true);
    ui.status(s.fetching_versions);
    let result = (|| -> Result<(), String> {
        let python = Pip::find_python().ok_or(s.python_not_found)?;
        let server = match server {
            Some(s) => s,
            None => get_working_pip_server(&ui).ok_or(i18n::strings().no_pip_server_err)?,
        };
        let mut pip = Pip::new(python, &server);
        let pkg = &app_config::get().install_packages[0];
        let versions = pip.list_versions(pkg, true)?;
        if versions.is_empty() {
            return Err(s.no_versions_net.into());
        }
        if let Some(local) = pip.local_version(pkg) {
            ui.log(&i18n::fill(s.current_version, &[&local.raw]));
        }
        ui.versions(versions.iter().map(|v| v.raw.clone()).collect());
        ui.log(s.versions_listed);
        Ok(())
    })();
    if let Err(e) = result {
        ui.log(&i18n::fill(s.error_prefix, &[&e]));
        logging::error(&e);
    }
    ui.busy(false);
}

/// 安装用户在恢复模式中选择的版本
/// 恢复模式：安装所选版本（UI 入口已移除，流程代码保留以备后用）
#[allow(dead_code)]
fn run_install_selected(ui: Arc<Ui>, version: String) {
    let s = i18n::strings();
    ui.busy(true);
    ui.status(&i18n::fill(s.installing_version_status2, &[&version]));
    let result = (|| -> Result<(), String> {
        let python = Pip::find_python().ok_or(s.python_not_found)?;
        let server = get_working_pip_server(&ui).ok_or(i18n::strings().no_pip_server_err)?;
        let mut pip = Pip::new(python, &server);
        {
            let ui2 = Arc::clone(&ui);
            pip.on_line = Arc::new(Mutex::new(Box::new(move |line| ui2.log(line))));
        }
        attach_progress(&mut pip, &ui);
        install_packages_version(&mut pip, &ui, &version)?;
        ui.log(&i18n::fill(s.installed_restart, &[&app_config::get().app_name]));
        Ok(())
    })();
    if let Err(e) = result {
        ui.log(&i18n::fill(s.error_prefix, &[&e]));
        logging::error(&e);
    }
    ui.busy(false);
}

/// 在后台线程执行主流程。
pub fn spawn_worker(ui: Arc<Ui>, f: impl FnOnce(Arc<Ui>) + Send + 'static) {
    std::thread::spawn(move || f(ui));
}


/// 取主窗口的 Win32 HWND：枚举本进程的可见顶层窗口（不能用按标题的
/// FindWindowW——多实例并存时会拿到别的实例的窗口）
#[cfg(windows)]
pub fn hwnd_of() -> Option<*mut core::ffi::c_void> {
    hwnd_of_excluding(None)
}

/// 同 [`hwnd_of`]，但可排除一个已知窗口（如主窗口，用于定位新弹出的对话框）
#[cfg(windows)]
pub fn hwnd_of_excluding(exclude: Option<*mut core::ffi::c_void>) -> Option<*mut core::ffi::c_void> {
    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(callback: extern "system" fn(*mut core::ffi::c_void, isize) -> i32, lparam: isize) -> i32;
        fn GetWindowThreadProcessId(hwnd: *mut core::ffi::c_void, pid: *mut u32) -> u32;
        fn IsWindowVisible(hwnd: *mut core::ffi::c_void) -> i32;
        fn GetCurrentProcessId() -> u32;
    }
    struct Ctx {
        exclude: Option<*mut core::ffi::c_void>,
        found: Option<*mut core::ffi::c_void>,
    }
    extern "system" fn enum_proc(hwnd: *mut core::ffi::c_void, lparam: isize) -> i32 {
        let ctx = unsafe { &mut *(lparam as *mut Ctx) };
        unsafe {
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == GetCurrentProcessId()
                && IsWindowVisible(hwnd) != 0
                && Some(hwnd) != ctx.exclude
                && !is_excluded_window(hwnd)
            {
                ctx.found = Some(hwnd);
                return 0; // 找到即停
            }
        }
        1
    }
    // 排除两类非主窗口：
    // 1. winit 跨线程事件派发辅助窗口（类名 "Winit Thread Event Target"，16x16），
    //    它可以通过 IsWindowVisible 过滤，必须显式排除，否则无边框/图标设置可能
    //    被错误地应用到它而非真正的主窗口上。
    // 2. 原生 MessageBox / 对话框（类名 "#32770"）：部署确认框在主窗口惰性创建
    //    之前就可能弹出，若不排除，无边框代码会 attach 到消息框上。
    unsafe fn is_excluded_window(hwnd: *mut core::ffi::c_void) -> bool {
        #[link(name = "user32")]
        extern "system" {
            fn GetClassNameW(hwnd: *mut core::ffi::c_void, buf: *mut u16, max: i32) -> i32;
        }
        let mut buf = [0u16; 64];
        GetClassNameW(hwnd, buf.as_mut_ptr(), 64);
        let class = String::from_utf16_lossy(&buf);
        let class = class.trim_end_matches('\0');
        class == "Winit Thread Event Target" || class == "#32770"
    }
    let mut ctx = Ctx { exclude, found: None };
    unsafe { EnumWindows(enum_proc, &mut ctx as *mut _ as isize); }
    ctx.found
}

/// 把工作目录切到 exe 所在目录。
///
/// 主程序与日志都按相对路径读写（`conf/`、`logs/`、`python-{ver}/`），
/// 而快捷方式、终端、其他程序的启动 cwd 未必是 exe 目录，必须显式对齐。
/// GUI 版与 CLI 版放在同一目录时，因此天然共享同一套运行环境。
///
/// :return: 成功时返回 `()`。
/// :raises String: 无法获取 exe 路径或切目录失败时。
pub fn chdir_to_exe() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe
        .parent()
        .ok_or_else(|| "无法获取程序所在目录".to_string())?;
    std::env::set_current_dir(dir).map_err(|e| e.to_string())
}
