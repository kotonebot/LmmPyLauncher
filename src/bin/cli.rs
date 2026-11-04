//! LmmPyLauncher CLI 版入口（console 子系统）。
//!
//! 定位是"纯启动器"：**不承担任何环境准备**——不探测镜像、不检查/安装更新、不部署
//! Python、不询问地区。只检查解释器与包是否就绪，然后原样把参数交给主程序；环境
//! 未就绪时报错并提示先运行 GUI 版。
//!
//! 与 GUI 版的共同点：同一套配置、同一份安装目录（`python-{ver}/`、`conf/`、`logs/`）。
//! 区别：无窗口、子进程继承当前控制台、参数与退出码透传。

// 控制台程序：windows 启动时自动分配/继承控制台，输出与退出码对调用方有效
#![windows_subsystem = "console"]

use std::io::Write;

use lmm_launcher::host::{self, Host};
use lmm_launcher::settings::Region;
use lmm_launcher::{
    chdir_to_exe, i18n, job, logging, parse_args, run_cli, Mode, Ui, UiMsg, BOOTSTRAP_VERSION,
};

/// CLI 版交互实现：全部走 stdin / stderr。
struct CliHost;

impl Host for CliHost {
    fn ask_region(&self) -> Region {
        ask_region_cli()
    }

    fn confirm(&self, text: &str, title: &str) -> bool {
        confirm_cli(text, title)
    }

    /// CLI 没有消息循环，退出由 `main` 的返回值决定
    fn request_quit(&self) {}

    fn crash(&self, title: &str, hint: &str, tail: &str) {
        eprintln!("\n{title}");
        eprintln!("{hint}");
        if !tail.is_empty() {
            eprintln!("---------- 主程序输出 ----------");
            eprintln!("{tail}");
            eprintln!("--------------------------------");
        }
    }

    fn feedback_hint(&self, _default: &str) -> String {
        // CLI 没有窗口可截图，只能让用户附终端输出与 logs 目录
        "如需反馈，请附上本次终端输出，并压缩本目录下的 logs 文件夹一并发送给开发者".to_string()
    }
}

fn main() {
    let s = i18n::strings();

    // Ctrl+C 会广播给同一控制台的所有进程：本进程忽略它、继续等待子进程结束，
    // 由 Python 侧自行处理 KeyboardInterrupt 并给出退出码
    #[cfg(windows)]
    ignore_ctrl_c();

    // 与 GUI 版一致：把 cwd 切到 exe 目录，从而共享 conf/、logs/、python-{ver}/
    if let Err(e) = chdir_to_exe() {
        eprintln!("{}", i18n::fill(s.set_cwd_fail, &[&e]));
        std::process::exit(1);
    }

    host::install(Box::new(CliHost));

    let args = match parse_args(Mode::Cli) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    logging::init().ok();
    logging::info(s.launcher_started);
    logging::info(&i18n::fill(s.launcher_version, &[BOOTSTRAP_VERSION]));
    // 创建 Job Object：启动器退出时子进程一并终止
    #[cfg(windows)]
    job::init();

    let (ui, rx) = Ui::new();

    // 消息泵：主程序启动前后的状态与日志转成 stderr 输出。
    // 需要静默时可用 2> 重定向。
    std::thread::spawn(move || {
        while let Ok(msg) = rx.recv() {
            match msg {
                UiMsg::Log(line) => eprintln!("{line}"),
                UiMsg::Status(status) => eprintln!("==> {status}"),
                // 进度条与忙碌状态只对 GUI 有意义
                UiMsg::Progress(_) | UiMsg::Busy(_) | UiMsg::Versions(_) => {}
            }
        }
    });

    // CLI 版不走安装/更新流程：只检查环境是否就绪，然后直接启动主程序。
    // 环境准备（Python 部署、pip 安装、自动更新）一律由 GUI 版负责。
    let code = match run_cli(&ui, &args) {
        // 主程序退出码原样透传，便于脚本判断成败
        Ok(code) => code,
        Err(e) => {
            eprintln!("\n{e}");
            1
        }
    };
    std::process::exit(code);
}

/// 首次运行询问地区。无 stdin（双击启动）时默认中国大陆，避免永久阻塞。
fn ask_region_cli() -> Region {
    let s = i18n::strings();
    eprintln!("{}", s.region_instruction);
    eprintln!("  [1] {}", s.region_cn);
    eprintln!("  [2] {}", s.region_intl);
    eprint!("> ");
    let _ = std::io::stdout().flush();

    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
        // 非交互环境（如从资源管理器双击）读不到输入，静默取默认值
        eprintln!("{}", s.region_cn);
        return Region::Cn;
    }
    match line.trim() {
        "2" => Region::Intl,
        _ => Region::Cn,
    }
}

/// 终端内确认（当前用于 embed Python 部署）。默认否，符合危险操作惯例。
fn confirm_cli(text: &str, title: &str) -> bool {
    eprintln!("{title}");
    eprintln!("{text}");
    eprint!("[y/N] ");
    let _ = std::io::stdout().flush();

    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
        return false;
    }
    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// 让本进程忽略 Ctrl+C：事件仍会发给同一控制台上的主程序。
#[cfg(windows)]
fn ignore_ctrl_c() {
    #[link(name = "kernel32")]
    extern "system" {
        fn SetConsoleCtrlHandler(
            handler: Option<extern "system" fn(u32) -> i32>,
            add: i32,
        ) -> i32;
    }
    /// 返回 1（TRUE）表示事件已被处理，系统不再终止本进程
    extern "system" fn handler(_ctrl_type: u32) -> i32 {
        1
    }
    unsafe {
        SetConsoleCtrlHandler(Some(handler), 1);
    }
}
