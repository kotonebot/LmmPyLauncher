//! LmmPyLauncher GUI 版入口（windows 子系统）。
//!
//! 负责：Slint splash 窗口、无边框/DWM 样式、消息泵（`UiMsg` → Slint）、
//! 崩溃报告窗口、首次运行的地区选择对话框。
//! 安装/更新/启动等逻辑全部在 `lmm_launcher` 库中，本文件只做 UI 层。

// 纯 GUI 程序：不分配控制台窗口（日志写入 logs/ 下的文件）
#![windows_subsystem = "windows"]

slint::include_modules!();

use std::sync::atomic::Ordering;
use std::sync::Arc;

use slint::SharedString;

use lmm_launcher::host::{self, Host};
use lmm_launcher::settings::Region;
use lmm_launcher::{
    app_config, chdir_to_exe, hwnd_of, i18n, job, logging, parse_args, run_main, spawn_worker,
    Mode, Ui, UiMsg, BOOTSTRAP_VERSION,
};

/// splash logo：编译期由 build.rs 从 launcher.json 的 logoPath 注入路径（无则空文件）
const LOGO_DATA: &[u8] = include_bytes!(env!("LMM_LOGO_PATH"));

/// GUI 版交互实现：全部走原生窗口 / Slint 对话框。
struct GuiHost;

impl Host for GuiHost {
    fn ask_region(&self) -> Region {
        ask_region_gui()
    }

    fn confirm(&self, text: &str, title: &str) -> bool {
        const MB_OKCANCEL: u32 = 0x0000_0001;
        const MB_ICONQUESTION: u32 = 0x0000_0020;
        const IDOK: i32 = 1;
        // 挂主窗口为属主：此时主窗口是前台，无属主的消息框会被它完全遮挡
        lmm_launcher::win32::message_box_owned(
            hwnd_of(),
            text,
            title,
            MB_OKCANCEL | MB_ICONQUESTION,
        ) == IDOK
    }

    fn request_quit(&self) {
        let _ = slint::quit_event_loop();
    }

    fn crash(&self, title: &str, hint: &str, tail: &str) {
        let title = title.to_string();
        let hint = hint.to_string();
        let tail = tail.to_string();
        let _ = slint::invoke_from_event_loop(move || {
            if let Err(e) = show_crash_window(&title, &hint, &tail) {
                logging::error(&i18n::fill(i18n::strings().crash_window_fail, &[&e]));
            }
        });
    }
}

fn main() {
    let s = i18n::strings();

    // 把工作目录切到 exe 所在目录：主程序及日志文件均按相对路径读写
    // （conf/、resources/、logs/），快捷方式/终端启动时 cwd 可能不是 exe 目录
    if let Err(e) = chdir_to_exe() {
        let msg = i18n::fill(s.set_cwd_fail, &[&e]);
        #[cfg(windows)]
        lmm_launcher::win32::message_box(&msg, "Error", 0x0 | 0x10); // MB_OK | MB_ICONERROR
        #[cfg(not(windows))]
        eprintln!("{msg}");
        std::process::exit(1);
    }

    host::install(Box::new(GuiHost));

    let args = match parse_args(Mode::Gui) {
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

    let (ui, rx) = Ui::new();

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
                            lmm_launcher::BUSY.store(b, Ordering::Relaxed);
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
            lmm_launcher::win32::confirm_exit_and_quit(
                lmm_launcher::BUSY.load(Ordering::Relaxed),
            );
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
                    lmm_launcher::win32::setup_frameless(hwnd);
                    lmm_launcher::win32::set_window_icon(hwnd);
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
    lmm_launcher::BUSY.store(true, Ordering::Relaxed);
    {
        let ui = Arc::clone(&ui);
        let args = args;
        spawn_worker(ui, move |ui| match run_main(ui, args, Mode::Gui) {
            // 主程序正常退出：启动器跟随退出
            Ok(o) if o.code == 0 => {
                let _ = slint::quit_event_loop();
            }
            // 主程序异常退出：弹崩溃报告，用户关闭后启动器退出
            Ok(o) => {
                let s = i18n::strings();
                let name = app_config::get().app_name.clone();
                let title = i18n::fill(s.crash_title, &[&name]);
                let hint = i18n::fill(s.crash_hint, &[&name, &o.code.to_string()]);
                host::get().crash(&title, &hint, &o.tail);
            }
            // 安装/更新/启动阶段失败：run_main 已把致命错误写入 UI 日志，
            // 保持窗口让用户查看日志与反馈提示
            Err(_) => {}
        });
    }

    run_event_loop(main_window);
}

/// 运行 Slint 事件循环。
fn run_event_loop(window: MainWindow) {
    window.run().expect(i18n::strings().event_loop_fail);
}

/// 崩溃报告窗口（在 UI 线程调用）。用户点确定后启动器整体退出。
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

/// 首次启动：弹地区选择窗口，阻塞工作线程直到用户选择。
fn ask_region_gui() -> Region {
    let (tx, rx) = std::sync::mpsc::channel::<Region>();
    let _ = slint::invoke_from_event_loop(move || {
        if let Err(e) = show_region_window(tx.clone()) {
            logging::error(&format!("无法显示地区选择窗口: {e}"));
        }
    });
    // 用户未选择就关闭事件循环（极少见）时默认 intl
    rx.recv().unwrap_or(Region::Intl)
}

fn show_region_window(tx: std::sync::mpsc::Sender<Region>) -> Result<(), String> {
    use slint::SharedString;
    let s = i18n::strings();
    // 主窗口先于本对话框存在，记录为模态属主
    let owner = hwnd_of();
    let w = RegionWindow::new().map_err(|e| e.to_string())?;
    // Task Dialog 惯例：原生标题栏标题 = 应用名，主指令才是提问文案
    w.set_window_title(SharedString::from(app_config::get().app_name.as_str()));
    w.set_instruction(SharedString::from(s.region_instruction));
    w.set_body(SharedString::from(s.region_body));
    w.set_label_cn(SharedString::from(s.region_cn));
    w.set_label_intl(SharedString::from(s.region_intl));
    w.set_note_cn(SharedString::from(s.region_cn_note));
    w.set_note_intl(SharedString::from(s.region_intl_note));
    {
        let tx = tx.clone();
        let weak = w.as_weak();
        w.on_choose_cn(move || {
            let _ = tx.send(Region::Cn);
            if let Some(o) = owner {
                lmm_launcher::win32::release_modal_dialog(o);
            }
            if let Some(w) = weak.upgrade() {
                let _ = w.hide();
            }
        });
    }
    {
        let weak = w.as_weak();
        w.on_choose_intl(move || {
            let _ = tx.send(Region::Intl);
            if let Some(o) = owner {
                lmm_launcher::win32::release_modal_dialog(o);
            }
            if let Some(w) = weak.upgrade() {
                let _ = w.hide();
            }
        });
    }
    // 模态选择框不允许关闭（没有有效选择就没有持久化配置）：
    // 点原生标题栏 X 一律忽略
    w.window()
        .on_close_requested(|| slint::CloseRequestResponse::KeepWindowShown);

    w.show().map_err(|e| e.to_string())?;

    // Slint/winit 没有模态窗口 API，走 Win32 原生机制（系统对话框同款）：
    // owned window + EnableWindow 禁用属主 + 居中到属主。Slint 窗口惰性创建，
    // 用 Timer 轮询等 HWND 出现（对齐主窗口的做法）
    if let Some(owner) = owner {
        let timer = std::rc::Rc::new(slint::Timer::default());
        let timer2 = timer.clone();
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(1),
            move || {
                if let Some(dialog) = lmm_launcher::hwnd_of_excluding(Some(owner)) {
                    timer2.stop();
                    lmm_launcher::win32::setup_modal_dialog(dialog, owner);
                    lmm_launcher::win32::disable_close_button(dialog);
                }
            },
        );
        // Timer 被 drop 会停止，泄漏一份活到对话框关闭
        std::mem::forget(timer);
    }
    Ok(())
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
    window
        .window()
        .set_position(slint::WindowPosition::Physical(slint::PhysicalPosition::new(x, y)));
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

/// 最小化窗口（Slint 1.18 没有暴露 minimize API，走 Win32）
#[cfg(windows)]
fn minimize_window(_window: &MainWindow) {
    #[link(name = "user32")]
    extern "system" {
        fn ShowWindow(hwnd: *mut core::ffi::c_void, n_cmd_show: i32) -> i32;
    }
    const SW_MINIMIZE: i32 = 6;
    if let Some(hwnd) = hwnd_of() {
        unsafe {
            ShowWindow(hwnd, SW_MINIMIZE);
        }
    }
}

#[cfg(not(windows))]
fn minimize_window(_window: &MainWindow) {}
