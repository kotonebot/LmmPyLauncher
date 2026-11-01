//! LmmPyLauncher — 通用 Python 桌面应用启动器（Rust + Slint GUI 版）
//!
//! 职责：
//! 1. 首次启动询问地区（决定 PyPI / Python 发行版下载源）
//! 2. 寻找最快的 PyPI 镜像源
//! 3. 安装与更新 pip 和主程序（包名来自编译期配置 launcher.json）
//! 4. 启动主程序
//! 5. 恢复模式（手动选择版本安装，流程保留）

// 纯 GUI 程序：不分配控制台窗口（日志写入 logs/ 下的文件）
#![windows_subsystem = "windows"]

mod app_config;
mod deploy;
mod i18n;
#[cfg(windows)]
mod job;
mod logging;
mod pip;
mod settings;
#[cfg(windows)]
mod win32;
mod version;

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::Stdio;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};

use slint::{SharedString, Weak};

use crate::pip::{HideConsoleWindow, Pip};
use crate::settings::Region;

const BOOTSTRAP_VERSION: &str = env!("CARGO_PKG_VERSION");
/// splash logo：编译期由 build.rs 从 launcher.json 的 logoPath 注入路径（无则空文件）
const LOGO_DATA: &[u8] = include_bytes!(env!("LMM_LOGO_PATH"));

/// 安装/升级进行中标志：WM_CLOSE 的退出确认需要跨模块读取（win32::wnd_proc）
pub(crate) static BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

slint::include_modules!();

/// UI 事件（工作线程 → UI 线程）
enum UiMsg {
    Log(String),
    Status(String),
    Progress(f32),
    Busy(bool),
    /// 恢复模式 UI 已移除，但恢复流程代码保留，消息通道维持不变
    #[allow(dead_code)]
    Versions(Vec<String>),
}

struct Ui {
    #[allow(dead_code)]
    weak: Weak<MainWindow>,
    tx: Sender<UiMsg>,
}

impl Ui {
    fn log(&self, line: &str) {
        let _ = self.tx.send(UiMsg::Log(line.to_string()));
    }
    fn status(&self, s: &str) {
        let _ = self.tx.send(UiMsg::Status(s.to_string()));
    }
    fn progress(&self, p: f32) {
        let _ = self.tx.send(UiMsg::Progress(p));
    }
    fn busy(&self, b: bool) {
        let _ = self.tx.send(UiMsg::Busy(b));
    }
    /// 恢复模式 UI 已移除，恢复流程保留以备后用
    #[allow(dead_code)]
    fn versions(&self, v: Vec<String>) {
        let _ = self.tx.send(UiMsg::Versions(v));
    }
}

/// 命令行参数
#[derive(Clone)]
struct Args {
    install_version: Option<String>,
    install_from_zip: Option<std::path::PathBuf>,
    install_from_package: Option<std::path::PathBuf>,
    skip_update: bool,
    extra_args: Vec<String>,
}

fn parse_args() -> Result<Args, String> {
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
        } else if a.starts_with("--") {
            // 未知开关原样传给主程序
            args.extra_args.push(a.clone());
        } else {
            // 位置参数：包文件
            let lower = a.to_ascii_lowercase();
            if lower.ends_with(".zip") {
                if args.install_from_zip.is_none() {
                    args.install_from_zip = Some(std::path::PathBuf::from(a));
                }
            } else if lower.ends_with(".whl") || lower.ends_with(".tar.gz") {
                if args.install_from_package.is_none() {
                    args.install_from_package = Some(std::path::PathBuf::from(a));
                }
            } else {
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

/// 运行主程序，阻塞直到其退出；正常退出时启动器跟随退出，
/// 异常退出时弹出崩溃报告窗口（展示输出尾部）
fn run_app(ui: &Ui, extra_args: &[String]) -> Result<(), String> {
    let s = i18n::strings();
    let config = app_config::get();
    let python = Pip::find_python().ok_or(s.python_not_found)?;

    ui.status(&i18n::fill(s.waiting_start, &[&config.app_name]));

    // 按编译期配置的命令模板组装启动命令（launcher.json → launch_command）
    let mut parts: Vec<String> = Vec::new();
    for tok in config.launch_command.split_whitespace() {
        match tok {
            "{python}" => parts.push(python.to_string_lossy().into_owned()),
            "{extra_args}" => parts.extend(extra_args.iter().cloned()),
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
        .env("PYTHONUNBUFFERED", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .hide_console_window();
    let mut child = cmd.spawn().map_err(|e| i18n::fill(s.spawn_fail, &[&e.to_string()]))?;
    // 挂入 Job Object：启动器退出时主程序一并被终止
    job::assign_child(&child);

    // 等待主程序出现第一个可见顶层窗口后再隐藏启动器（最多 30s）。
    // 若在窗口出现前就退出（启动即崩），不隐藏，直接走崩溃报告流程
    #[cfg_attr(windows, allow(unused_assignments))]
    #[cfg(windows)]
    let mut appeared = false;
    #[cfg(not(windows))]
    let mut appeared = true;
    #[cfg(windows)]
    {
        appeared = false;
        let pid = child.id();
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
    }
    if appeared {
        // 主程序窗口已接管前台：隐藏启动器窗口
        #[cfg(windows)]
        hide_launcher_window();
    }

    // 收集主程序输出尾部（stdout+stderr 合计最近 100 行），供崩溃提示窗口展示
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
    if code == 0 {
        ui.progress(100.0);
        ui.status(&i18n::fill(s.app_exited, &[&config.app_name]));
        ui.log(&i18n::fill(s.app_exited, &[&config.app_name]));
        // 正常退出：启动器跟着退出
        let _ = slint::quit_event_loop();
        return Ok(());
    }

    // 异常退出：弹出崩溃报告窗口展示输出尾部，用户关闭后启动器退出
    let tail_text = tail.lock().unwrap().join("\n");
    let title = i18n::fill(s.crash_title, &[&config.app_name]);
    let hint = i18n::fill(s.crash_hint, &[&config.app_name, &code.to_string()]);
    ui.log(&i18n::fill(s.crash_exit_log, &[&code.to_string()]));
    logging::error(&i18n::fill(s.crash_exit_log, &[&code.to_string()]));
    {
        let title = title.clone();
        let hint = hint.clone();
        let tail_text = tail_text.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Err(e) = show_crash_window(&title, &hint, &tail_text) {
                logging::error(&i18n::fill(i18n::strings().crash_window_fail, &[&e]));
            }
        });
    }
    Err(i18n::fill(s.crash_exit_log, &[&code.to_string()]))
}

/// 崩溃报告窗口（在 UI 线程调用）。用户点确定后启动器整体退出。
#[cfg(windows)]
fn show_crash_window(title: &str, hint: &str, text: &str) -> Result<(), String> {
    let s = i18n::strings();
    let w = CrashWindow::new().map_err(|e| e.to_string())?;
    w.set_window_title(SharedString::from(title));
    w.set_hint(SharedString::from(hint));
    w.set_crash_text(SharedString::from(text));
    w.set_label_ok(SharedString::from(s.ok_btn));
    w.on_dismiss(move || {
        let _ = slint::quit_event_loop();
    });
    w.show().map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn show_crash_window(_t: &str, _h: &str, _x: &str) -> Result<(), String> {
    Ok(())
}

/// 隐藏启动器主窗口（主程序运行期间不再需要它）
#[cfg(windows)]
fn hide_launcher_window() {
    #[link(name = "user32")]
    extern "system" {
        fn ShowWindow(hwnd: *mut core::ffi::c_void, n_cmd_show: i32) -> i32;
    }
    const SW_HIDE: i32 = 0;
    if let Some(hwnd) = hwnd_of() {
        unsafe { ShowWindow(hwnd, SW_HIDE); }
    }
}

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

/// 主流程（在后台线程执行）
fn run_main(ui: Arc<Ui>, args: Args) {
    let s = i18n::strings();
    ui.busy(true);
    ui.progress(0.0);
    ui.status(s.initializing);

    let result = (|| -> Result<(), String> {
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
        run_app(&ui, &args.extra_args)?;
        Ok(())
    })();

    match result {
        Ok(()) => {}
        Err(e) => {
            ui.status(&i18n::fill(s.fatal_error, &[&e]));
            ui.log(&i18n::fill(s.fatal_error, &[&e]));
            ui.log(s.feedback_hint);
            logging::error(&e);
        }
    }
    ui.busy(false);
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

fn spawn_worker(ui: Arc<Ui>, f: impl FnOnce(Arc<Ui>) + Send + 'static) {
    std::thread::spawn(move || f(ui));
}

fn main() {
    let s = i18n::strings();
    // 把工作目录切到 exe 所在目录：主程序及日志文件均按相对路径读写
    // （conf/、resources/、logs/），快捷方式/终端启动时 cwd 可能不是 exe 目录
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
    {
        if let Err(e) = std::env::set_current_dir(&dir) {
            let msg = i18n::fill(s.set_cwd_fail, &[&e.to_string()]);
            #[cfg(windows)]
            crate::win32::message_box(&msg, "Error", 0x0 | 0x10); // MB_OK | MB_ICONERROR
            #[cfg(not(windows))]
            eprintln!("{msg}");
            std::process::exit(1);
        }
    }

    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    logging::init().ok();
    logging::info(s.launcher_started);
    logging::info(&i18n::fill(s.launcher_version, &[BOOTSTRAP_VERSION]));
    // 创建 Job Object：此后 spawn 的子进程全部挂入，启动器退出时一并终止
    #[cfg(windows)]
    job::init();

    let (tx, rx) = channel::<UiMsg>();

    // 处理跳过更新的标志（特殊安装或 --skip-update 时不检查更新）
    let mut args = args;
    if args.install_version.is_some()
        || args.install_from_zip.is_some()
        || args.install_from_package.is_some()
        || args.skip_update
    {
        args.skip_update = true;
    }

    let main_window = MainWindow::new().expect(s.window_create_fail);
    let config = app_config::get();
    main_window.set_app_name(SharedString::from(config.app_name.as_str()));
    // 未配置 logoPath 时是空文件，跳过解码（避免 slint 打印解码错误），
    // 用空图片（宽度 0），Slint 端不渲染
    main_window.set_logo(if LOGO_DATA.is_empty() {
        slint::Image::default()
    } else {
        slint::Image::load_from_data(LOGO_DATA, None).unwrap_or_default()
    });
    main_window.set_status(SharedString::from(s.ready));
    main_window.set_label_details(SharedString::from(s.details));
    main_window.set_label_collapse_details(SharedString::from(s.collapse_details));

    let weak = main_window.as_weak();
    let ui = Arc::new(Ui { weak, tx });

    // UI 事件泵：把工作线程的消息转发到 UI
    {
        let weak = main_window.as_weak();
        std::thread::spawn(move || {
            while let Ok(msg) = rx.recv() {
                let weak = weak.clone();
                slint::invoke_from_event_loop(move || {
                    let Some(w) = weak.upgrade() else { return };
                    match msg {
                        UiMsg::Log(line) => {
                            let existing = w.get_log_text();
                            w.set_log_text(SharedString::from(format!("{existing}{line}\n")));
                        }
                        UiMsg::Status(s) => w.set_status(SharedString::from(s)),
                        UiMsg::Progress(p) => w.set_progress(p),
                        UiMsg::Busy(b) => {
                            BUSY.store(b, std::sync::atomic::Ordering::Relaxed);
                            w.set_busy(b);
                        }
                        // 恢复模式 UI 已移除；Versions 消息保留以兼容恢复流程代码
                        UiMsg::Versions(_) => {}
                    }
                })
                .ok();
            }
        });
    }

    {
        let weak = main_window.as_weak();
        main_window.on_minimize_window(move || {
            if let Some(w) = weak.upgrade() {
                minimize_window(&w);
            }
        });
    }
    {
        main_window.on_close_window(move || {
            // 安装/升级进行中先确认（标题栏关闭按钮入口）；与 WM_CLOSE 共用逻辑
            #[cfg(windows)]
            win32::confirm_exit_and_quit(BUSY.load(std::sync::atomic::Ordering::Relaxed));
            #[cfg(not(windows))]
            let _ = slint::quit_event_loop();
        });
    }

    // Windows：先放到屏幕外，无边框样式应用完成后再移入屏幕中央（见下方 Timer）
    #[cfg(windows)]
    place_offscreen(&main_window);

    main_window.show().expect(s.window_show_fail);
    #[cfg(not(windows))]
    center_on_screen(&main_window);

    // 无边框窗口 + DWM 接入。Slint/winit 的原生窗口在事件循环启动后才创建
    // （WinitWindowOrNone::ensure_window 惰性创建），所以用 Timer 轮询等待窗口出现。
    // 窗口先被放到屏幕外（place_offscreen），样式应用完成后再移入屏幕中央，
    // 避免原生标题栏先闪现一帧。
    #[cfg(windows)]
    {
        let timer = std::rc::Rc::new(slint::Timer::default());
        let timer2 = timer.clone();
        let weak = main_window.as_weak();
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(1),
            move || {
                if let Some(hwnd) = hwnd_of() {
                    timer2.stop();
                    logging::info("找到主窗口 HWND，应用无边框/DWM 设置");
                    win32::setup_frameless(hwnd);
                    win32::set_window_icon(hwnd);
                    if let Some(w) = weak.upgrade() {
                        center_on_screen(&w);
                    }
                }
            },
        );
        // Slint Timer 被 drop 时会停止，这里泄漏一份句柄让它活到进程结束
        std::mem::forget(timer);
    }

    // 启动时自动执行主流程
    // BUSY 初始为 true：默认流程全程 busy，避免启动初期 WM_CLOSE 绕过确认
    BUSY.store(true, std::sync::atomic::Ordering::Relaxed);
    {
        let ui = Arc::clone(&ui);
        let args = args.clone();
        spawn_worker(ui, move |ui| run_main(ui, args));
    }

    run_main_window(main_window);
}

fn run_main_window(window: MainWindow) {
    window.run().expect(i18n::strings().event_loop_fail);
}

/// 将窗口放到主屏幕正中央（Slint 没有内置居中，用 Win32 API 计算）
#[cfg(windows)]
fn center_on_screen(window: &MainWindow) {
    #[link(name = "user32")]
    extern "system" {
        fn GetSystemMetrics(nIndex: i32) -> i32;
    }
    const SM_CXSCREEN: i32 = 0;
    const SM_CYSCREEN: i32 = 1;
    let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    if screen_w <= 0 || screen_h <= 0 {
        return;
    }
    let size = window.window().size();
    let x = ((screen_w - size.width as i32) / 2).max(0);
    let y = ((screen_h - size.height as i32) / 2).max(0);
    window.window().set_position(slint::WindowPosition::Physical(
        slint::PhysicalPosition::new(x, y),
    ));
}

#[cfg(not(windows))]
fn center_on_screen(_window: &MainWindow) {}

/// 把窗口放到屏幕外：无边框样式应用完成前不显示在屏幕上，避免原生标题栏闪现。
/// show() 前调用——Slint 会把位置存进 winit 的 WindowAttributes，窗口创建即生效。
#[cfg(windows)]
fn place_offscreen(window: &MainWindow) {
    window.window().set_position(slint::WindowPosition::Physical(
        slint::PhysicalPosition::new(-30_000, -30_000),
    ));
}

/// 取主窗口的 Win32 HWND：枚举本进程的可见顶层窗口（不能用按标题的
/// FindWindowW——多实例并存时会拿到别的实例的窗口）
#[cfg(windows)]
pub(crate) fn hwnd_of() -> Option<*mut core::ffi::c_void> {
    hwnd_of_excluding(None)
}

/// 同 [`hwnd_of`]，但可排除一个已知窗口（如主窗口，用于定位新弹出的对话框）
#[cfg(windows)]
pub(crate) fn hwnd_of_excluding(exclude: Option<*mut core::ffi::c_void>) -> Option<*mut core::ffi::c_void> {
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

/// 最小化窗口（Slint 1.18 没有暴露 minimize API，走 Win32）
#[cfg(windows)]
fn minimize_window(_window: &MainWindow) {
    #[link(name = "user32")]
    extern "system" {
        fn ShowWindow(hwnd: *mut core::ffi::c_void, n_cmd_show: i32) -> i32;
    }
    const SW_MINIMIZE: i32 = 6;
    if let Some(hwnd) = hwnd_of() {
        unsafe { ShowWindow(hwnd, SW_MINIMIZE); }
    }
}

#[cfg(not(windows))]
fn minimize_window(_window: &MainWindow) {}
