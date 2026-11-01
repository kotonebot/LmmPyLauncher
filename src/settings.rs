//! 运行时配置（区别于编译期 launcher.json）：exe 旁的 `launcher.conf.json`。
//!
//! 目前只有一项：`region`（"cn" | "intl"），决定 PyPI 镜像与 Python
//! 发行版下载源。首次启动（配置文件不存在）时弹出地区选择窗口，
//! 用户选择后持久化，此后不再询问。

use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// 中国大陆：使用国内 PyPI 镜像与 Python 发行版镜像
    Cn,
    /// 其他地区：直连 PyPI 官方源与 python.org
    Intl,
}

impl Region {
    fn as_str(self) -> &'static str {
        match self {
            Region::Cn => "cn",
            Region::Intl => "intl",
        }
    }
}

static REGION: OnceLock<Region> = OnceLock::new();

/// 已确认的地区。必须先在主流程开头调用 [`ensure`]。
pub fn get() -> Region {
    *REGION.get().expect(&*crate::i18n::fill(crate::i18n::strings().region_not_init, &[]))
}

/// 加载地区配置；没有有效配置时弹窗询问用户并持久化。
/// 可在任意线程调用（弹窗通过 invoke_from_event_loop 切到 UI 线程）。
pub fn ensure() -> Region {
    if let Some(r) = REGION.get() {
        return *r;
    }
    let region = load().unwrap_or_else(ask_user);
    let _ = REGION.set(region);
    save(region);
    crate::logging::info(&format!("地区: {}", region.as_str()));
    region
}

/// exe 所在目录
pub(crate) fn exe_dir() -> Result<PathBuf, String> {
    std::env::current_exe()
        .map_err(|e| format!("无法获取程序位置: {e}"))?
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "无法获取程序所在目录".into())
}

fn conf_path() -> Result<PathBuf, String> {
    Ok(exe_dir()?.join("launcher.conf.json"))
}

fn load() -> Option<Region> {
    let path = conf_path().ok()?;
    let text = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    match v["region"].as_str()? {
        "cn" => Some(Region::Cn),
        "intl" => Some(Region::Intl),
        _ => None,
    }
}

fn save(region: Region) {
    let Ok(path) = conf_path() else { return };
    let text = format!("{{\n  \"region\": \"{}\"\n}}\n", region.as_str());
    let _ = std::fs::write(path, text);
}

/// 首次启动：弹地区选择窗口，阻塞工作线程直到用户选择
fn ask_user() -> Region {
    let (tx, rx) = std::sync::mpsc::channel::<Region>();
    let _ = slint::invoke_from_event_loop(move || {
        if let Err(e) = show_region_window(tx.clone()) {
            crate::logging::error(&format!("无法显示地区选择窗口: {e}"));
        }
    });
    // 用户未选择就关闭事件循环（极少见）时默认 intl
    rx.recv().unwrap_or(Region::Intl)
}

#[cfg(windows)]
fn show_region_window(tx: std::sync::mpsc::Sender<Region>) -> Result<(), String> {
    use slint::SharedString;
    let s = crate::i18n::strings();
    // 主窗口先于本对话框存在，记录为模态属主
    let owner = crate::hwnd_of();
    let w = RegionWindow::new().map_err(|e| e.to_string())?;
    w.set_window_title(SharedString::from(s.region_title));
    w.set_question(SharedString::from(s.region_question));
    w.set_label_cn(SharedString::from(s.region_cn));
    w.set_label_intl(SharedString::from(s.region_intl));
    {
        let tx = tx.clone();
        let weak = w.as_weak();
        let owner = owner;
        w.on_choose_cn(move || {
            let _ = tx.send(Region::Cn);
            if let Some(o) = owner {
                crate::win32::release_modal_dialog(o);
            }
            if let Some(w) = weak.upgrade() {
                let _ = w.hide();
            }
        });
    }
    {
        let weak = w.as_weak();
        let owner = owner;
        w.on_choose_intl(move || {
            let _ = tx.send(Region::Intl);
            if let Some(o) = owner {
                crate::win32::release_modal_dialog(o);
            }
            if let Some(w) = weak.upgrade() {
                let _ = w.hide();
            }
        });
    }
    // 点原生标题栏 X 关闭也要释放属主，否则主窗口将永远不可聚焦
    // （on_close_requested 在 slint::Window 上，不是组件生成的回调）
    if let Some(owner) = owner {
        w.window().on_close_requested(move || {
            crate::win32::release_modal_dialog(owner);
            slint::CloseRequestResponse::HideWindow
        });
    }

    w.show().map_err(|e| e.to_string())?;

    // Slint/winit 没有模态窗口 API，走 Win32 原生机制（系统对话框同款）：
    // owned window + EnableWindow 禁用属主 + 居中到属主。Slint 窗口惰性创建，
    // 用 Timer 轮询等 HWND 出现（对齐 main.rs 的做法）
    if let Some(owner) = owner {
        let timer = std::rc::Rc::new(slint::Timer::default());
        let timer2 = timer.clone();
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(1),
            move || {
                if let Some(dialog) = crate::hwnd_of_excluding(Some(owner)) {
                    timer2.stop();
                    crate::win32::setup_modal_dialog(dialog, owner);
                }
            },
        );
        // Timer 被 drop 会停止，泄漏一份活到对话框关闭
        std::mem::forget(timer);
    }
    Ok(())
}

#[cfg(not(windows))]
fn show_region_window(_tx: std::sync::mpsc::Sender<Region>) -> Result<(), String> {
    Ok(())
}

slint::include_modules!();
