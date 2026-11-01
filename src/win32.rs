//! Windows 无边框窗口支持（移植自 kaa 本体 platform_win32.py 的方案）：
//! - 保留 WS_CAPTION/WS_THICKFRAME 样式，WM_NCCALCSIZE 返回 0 抹掉标准边框
//! - WM_NCHITTEST：边缘返回 resize HT*，标题栏返回 HTCAPTION（系统原生拖动、
//!   双击最大化），按钮区交给默认处理（HTCLIENT → Slint 正常收点击）
//! - DwmExtendFrameIntoClientArea 接入 DWM：原生阴影 + Win11 圆角，
//!   并按系统主题设置 immersive dark mode

#![cfg(windows)]

use std::sync::atomic::{AtomicIsize, Ordering};

#[link(name = "user32")]
extern "system" {
    fn SetWindowLongPtrW(hwnd: *mut core::ffi::c_void, index: i32, value: isize) -> isize;
    fn GetWindowLongPtrW(hwnd: *mut core::ffi::c_void, index: i32) -> isize;
    fn CallWindowProcW(prev: isize, hwnd: *mut core::ffi::c_void, msg: u32, wp: usize, lp: isize) -> isize;
    fn DefWindowProcW(hwnd: *mut core::ffi::c_void, msg: u32, wp: usize, lp: isize) -> isize;
    fn GetWindowRect(hwnd: *mut core::ffi::c_void, rect: *mut RECT) -> i32;
    fn GetClientRect(hwnd: *mut core::ffi::c_void, rect: *mut RECT) -> i32;
    fn IsZoomed(hwnd: *mut core::ffi::c_void) -> i32;
    fn GetSystemMetrics(index: i32) -> i32;
    fn GetDpiForWindow(hwnd: *mut core::ffi::c_void) -> u32;
    fn SetWindowPos(hwnd: *mut core::ffi::c_void, after: *mut core::ffi::c_void, x: i32, y: i32, w: i32, h: i32, flags: u32) -> i32;
}

#[link(name = "dwmapi")]
extern "system" {
    fn DwmExtendFrameIntoClientArea(hwnd: *mut core::ffi::c_void, margins: *const MARGINS) -> i32;
    fn DwmSetWindowAttribute(hwnd: *mut core::ffi::c_void, attr: u32, value: *const core::ffi::c_void, size: u32) -> i32;
}

#[repr(C)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct MARGINS {
    cx_left_width: i32,
    cx_right_width: i32,
    cy_top_height: i32,
    cy_bottom_height: i32,
}

const GWLP_WNDPROC: i32 = -4;
const GWL_STYLE: i32 = -16;
const GWL_HWNDPARENT: i32 = -8;
const WM_NCCALCSIZE: u32 = 0x0083;
const WM_NCHITTEST: u32 = 0x0084;
const WM_CLOSE: u32 = 0x0010;
const WS_CAPTION: isize = 0x00C0_0000;
const WS_THICKFRAME: isize = 0x0004_0000;
const WS_MAXIMIZEBOX: isize = 0x0001_0000;
const WS_MINIMIZEBOX: isize = 0x0002_0000;
const WS_SYSMENU: isize = 0x0008_0000;
const SWP_NOSIZE: u32 = 0x0001;
const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_FRAMECHANGED: u32 = 0x0020;

#[allow(dead_code)]
const HTCLIENT: isize = 1;
const HTCAPTION: isize = 2;
const HTLEFT: isize = 10;
const HTRIGHT: isize = 11;
const HTTOP: isize = 12;
const HTTOPLEFT: isize = 13;
const HTTOPRIGHT: isize = 14;
const HTBOTTOM: isize = 15;
const HTBOTTOMLEFT: isize = 16;
const HTBOTTOMRIGHT: isize = 17;

const SM_CXFRAME: i32 = 32;
const SM_CXPADDEDBORDER: i32 = 92;

const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;

/// 与 TitleBar 匹配的几何参数（逻辑像素）
const TITLE_BAR_H: i32 = 36;
const BTN_W: i32 = 44;
const RESIZE_EDGE: i32 = 4;

static ORIG_WNDPROC: AtomicIsize = AtomicIsize::new(0);

/// 原生消息框（可在任意线程调用）。返回按钮 ID（如 IDOK=1、IDCANCEL=2）。
pub fn message_box(text: &str, title: &str, style: u32) -> i32 {
    message_box_owned(None, text, title, style)
}

/// 带属主的消息框：owner 为 Some 时弹窗始终保持在属主之上并居中到属主。
/// 主窗口是前台时必须传入 owner，否则无属主弹窗抢不到前台，会被主窗口完全遮挡。
pub fn message_box_owned(
    owner: Option<*mut core::ffi::c_void>,
    text: &str,
    title: &str,
    style: u32,
) -> i32 {
    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(hwnd: *mut core::ffi::c_void, text: *const u16, title: *const u16, style: u32) -> i32;
    }
    let text: Vec<u16> = text.encode_utf16().chain([0]).collect();
    let title: Vec<u16> = title.encode_utf16().chain([0]).collect();
    unsafe {
        MessageBoxW(owner.unwrap_or(std::ptr::null_mut()), text.as_ptr(), title.as_ptr(), style)
    }
}

/// 无边框化 + DWM 接入。在窗口显示后、UI 线程调用。
pub fn setup_frameless(hwnd: *mut core::ffi::c_void) {
    unsafe {
        // 记录 Slint 期望的客户区尺寸（winit 按无边框客户区创建，
        // NCCALCSIZE=0 后客户区 == 窗口矩形，需把窗口缩回该尺寸避免布局被拉伸）
        let mut cr = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        GetClientRect(hwnd, &mut cr);
        let cw = cr.right - cr.left;
        let ch = cr.bottom - cr.top;

        // 保留 caption/thickframe 样式：DWM 才会画阴影和圆角，系统才有贴靠/动画
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        SetWindowLongPtrW(
            hwnd,
            GWL_STYLE,
            style | WS_CAPTION | WS_THICKFRAME | WS_MAXIMIZEBOX | WS_MINIMIZEBOX | WS_SYSMENU,
        );

        // DWM：边框扩展到整个客户区（-1 = 全窗口），得到阴影与圆角
        let margins = MARGINS { cx_left_width: -1, cx_right_width: -1, cy_top_height: -1, cy_bottom_height: -1 };
        DwmExtendFrameIntoClientArea(hwnd, &margins);

        // 标题栏文字/阴影颜色跟随系统深浅色
        set_immersive_dark_mode(hwnd);

        // 子类化：接管 WM_NCCALCSIZE / WM_NCHITTEST
        let orig = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, wnd_proc as *const () as isize);
        ORIG_WNDPROC.store(orig, Ordering::Relaxed);

        // 通知系统重新计算边框
        SetWindowPos(hwnd, std::ptr::null_mut(), 0, 0, 0, 0, SWP_NOSIZE | SWP_NOMOVE | SWP_NOZORDER | SWP_FRAMECHANGED);

        // 缩回 Slint 期望的客户区尺寸
        SetWindowPos(hwnd, std::ptr::null_mut(), 0, 0, cw, ch, SWP_NOMOVE | SWP_NOZORDER);
    }
}

/// 把 exe 资源中的图标设为窗口/任务栏图标（winresource 嵌入的资源 ID 为 1）
pub fn set_window_icon(hwnd: *mut core::ffi::c_void) {
    #[link(name = "user32")]
    extern "system" {
        fn LoadImageW(hinst: *mut core::ffi::c_void, name: *const u16, type_: u32, cx: i32, cy: i32, flags: u32) -> *mut core::ffi::c_void;
        fn SendMessageW(hwnd: *mut core::ffi::c_void, msg: u32, wparam: usize, lparam: isize) -> isize;
        fn GetModuleHandleW(name: *const u16) -> *mut core::ffi::c_void;
    }
    const IMAGE_ICON: u32 = 1;
    const LR_DEFAULTSIZE: u32 = 0x40;
    const WM_SETICON: u32 = 0x0080;
    const ICON_SMALL: usize = 0;
    const ICON_BIG: usize = 1;

    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        // MAKEINTRESOURCE(1)
        let hicon = LoadImageW(hinst, 1 as _, IMAGE_ICON, 0, 0, LR_DEFAULTSIZE);
        if hicon.is_null() {
            return;
        }
        SendMessageW(hwnd, WM_SETICON, ICON_SMALL, hicon as isize);
        SendMessageW(hwnd, WM_SETICON, ICON_BIG, hicon as isize);
    }
}

#[link(name = "advapi32")]
extern "system" {
    fn RegGetValueW(
        hkey: *mut core::ffi::c_void,
        sub_key: *const u16,
        value: *const u16,
        flags: u32,
        r#type: *mut u32,
        data: *mut core::ffi::c_void,
        len: *mut u32,
    ) -> i32;
}

const HKEY_CURRENT_USER: *mut core::ffi::c_void = 0x8000_0001usize as _;
const RRF_RT_REG_DWORD: u32 = 0x10;

fn set_immersive_dark_mode(hwnd: *mut core::ffi::c_void) {
    // 读 AppsUseLightTheme：0 = 系统深色
    let sub: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"
        .encode_utf16().chain([0]).collect();
    let name: Vec<u16> = "AppsUseLightTheme".encode_utf16().chain([0]).collect();
    let mut light: u32 = 1;
    let mut size: u32 = 4;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            sub.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            &mut light as *mut u32 as _,
            &mut size,
        );
        let dark: u32 = (light == 0) as u32;
        DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const u32 as _, 4);
    }
}

/// 退出确认：busy（安装/升级进行中）时弹原生确认框，确认后才退出 Slint 事件循环。
/// WM_CLOSE（系统关闭/Alt+F4，见 wnd_proc）与标题栏关闭按钮共用。
pub fn confirm_exit_and_quit(busy: bool) {
    if busy {
        let strings = crate::i18n::strings();
        let text = strings.confirm_exit.replace("{}", &crate::app_config::get().app_name);
        const MB_OKCANCEL: u32 = 0x1;
        const MB_ICONQUESTION: u32 = 0x20;
        const IDOK: i32 = 1;
        if message_box(&text, strings.confirm_exit_title, MB_OKCANCEL | MB_ICONQUESTION) != IDOK {
            return;
        }
    }
    let _ = slint::quit_event_loop();
}

unsafe extern "system" fn wnd_proc(hwnd: *mut core::ffi::c_void, msg: u32, wp: usize, lp: isize) -> isize {
    match msg {
        // 拦截系统关闭：winit 收到 WM_CLOSE 会直接退出事件循环，
        // 绕过 on_close_window（它只由标题栏关闭按钮触发），必须在此处理
        WM_CLOSE => {
            confirm_exit_and_quit(crate::BUSY.load(Ordering::Relaxed));
            0
        }
        // 抹掉标准边框：返回 0 让客户区覆盖整个窗口
        WM_NCCALCSIZE if wp != 0 => {
            if IsZoomed(hwnd) != 0 {
                // 最大化时保留系统边框宽度，避免内容顶出屏幕
                let border = GetSystemMetrics(SM_CXFRAME) + GetSystemMetrics(SM_CXPADDEDBORDER);
                let params = &mut *(lp as *mut [RECT; 3]);
                params[0].top += border;
            }
            0
        }
        WM_NCHITTEST => hit_test(hwnd, lp),
        _ => {
            let orig = ORIG_WNDPROC.load(Ordering::Relaxed);
            if orig != 0 {
                CallWindowProcW(orig, hwnd, msg, wp, lp)
            } else {
                DefWindowProcW(hwnd, msg, wp, lp)
            }
        }
    }
}

/// 移植自 kaa WindowEventFilter._hit_test：边缘 resize + 标题栏 HTCAPTION，
/// 右侧按钮区交还默认处理（Slint 正常接收点击）
unsafe fn hit_test(hwnd: *mut core::ffi::c_void, lp: isize) -> isize {
    let sx = (lp as i16) as i32;
    let sy = ((lp >> 16) as i16) as i32;

    let mut wr = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    GetWindowRect(hwnd, &mut wr);
    let wx = sx - wr.left;
    let wy = sy - wr.top;
    let ww = wr.right - wr.left;
    let wh = wr.bottom - wr.top;

    let scale = GetDpiForWindow(hwnd) as f64 / 96.0;
    let tb_h = (TITLE_BAR_H as f64 * scale).round() as i32;
    let border = (RESIZE_EDGE as f64 * scale).round() as i32;
    let btn_w = (BTN_W as f64 * scale).round() as i32;
    let is_max = IsZoomed(hwnd) != 0;

    if !is_max {
        let top = wy < border;
        let bottom = wy >= wh - border;
        let left = wx < border;
        let right = wx >= ww - border;
        if top && left { return HTTOPLEFT; }
        if top && right { return HTTOPRIGHT; }
        if bottom && left { return HTBOTTOMLEFT; }
        if bottom && right { return HTBOTTOMRIGHT; }
        if top { return HTTOP; }
        if bottom { return HTBOTTOM; }
        if left { return HTLEFT; }
        if right { return HTRIGHT; }
    }

    if wy < tb_h {
        // 最小化/关闭按钮区（右起各一个 BTN_W）：交还默认 → HTCLIENT → Slint 处理
        if wx >= ww - btn_w {
            return pass_through(hwnd, lp);
        }
        if wx >= ww - btn_w * 2 {
            return pass_through(hwnd, lp);
        }
        // 其余标题栏区域：系统原生拖动 + 双击最大化
        return HTCAPTION;
    }

    pass_through(hwnd, lp)
}

unsafe fn pass_through(hwnd: *mut core::ffi::c_void, lp: isize) -> isize {
    let orig = ORIG_WNDPROC.load(Ordering::Relaxed);
    if orig != 0 {
        CallWindowProcW(orig, hwnd, WM_NCHITTEST, 0, lp)
    } else {
        DefWindowProcW(hwnd, WM_NCHITTEST, 0, lp)
    }
}

/// 原生模态对话框化（Win32 标准机制，系统对话框同款）：
/// 1. 把 dialog 设为 owner 的 owned window——始终保持在 owner 之上、随 owner 最小化
/// 2. EnableWindow(owner, FALSE)——owner 关闭前无法被聚焦/点击
/// 3. 把 dialog 居中到 owner 上方并抢焦点
/// 在 dialog 显示后（HWND 已创建）调用一次。成对调用 [`release_modal_dialog`]。
pub fn setup_modal_dialog(dialog: *mut core::ffi::c_void, owner: *mut core::ffi::c_void) {
    #[link(name = "user32")]
    extern "system" {
        fn SetWindowLongPtrW(hwnd: *mut core::ffi::c_void, index: i32, value: isize) -> isize;
        fn EnableWindow(hwnd: *mut core::ffi::c_void, enable: i32) -> i32;
        fn SetForegroundWindow(hwnd: *mut core::ffi::c_void) -> i32;
    }
    unsafe {
        SetWindowLongPtrW(dialog, GWL_HWNDPARENT, owner as isize);
        EnableWindow(owner, 0);
        // 居中到 owner
        let mut or_ = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        let mut dr = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        GetWindowRect(owner, &mut or_);
        GetWindowRect(dialog, &mut dr);
        let dw = dr.right - dr.left;
        let dh = dr.bottom - dr.top;
        let x = or_.left + ((or_.right - or_.left) - dw) / 2;
        let y = or_.top + ((or_.bottom - or_.top) - dh) / 2;
        SetWindowPos(dialog, std::ptr::null_mut(), x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER);
        SetForegroundWindow(dialog);
    }
}

/// 置灰窗口标题栏的关闭按钮（X）：从系统菜单移除 SC_CLOSE。
/// 用于不允许关闭的模态对话框；配合 on_close_requested 的 KeepWindowShown 双保险。
pub fn disable_close_button(hwnd: *mut core::ffi::c_void) {
    #[link(name = "user32")]
    extern "system" {
        fn GetSystemMenu(hwnd: *mut core::ffi::c_void, revert: i32) -> *mut core::ffi::c_void;
        fn EnableMenuItem(menu: *mut core::ffi::c_void, item: u32, flags: u32) -> i32;
    }
    const SC_CLOSE: u32 = 0xF060;
    const MF_BYCOMMAND: u32 = 0x0;
    const MF_GRAYED: u32 = 0x1;
    unsafe {
        let menu = GetSystemMenu(hwnd, 0);
        if !menu.is_null() {
            EnableMenuItem(menu, SC_CLOSE, MF_BYCOMMAND | MF_GRAYED);
        }
    }
}

/// 关闭模态对话框时调用：重新启用属主窗口并交还前台
pub fn release_modal_dialog(owner: *mut core::ffi::c_void) {
    #[link(name = "user32")]
    extern "system" {
        fn EnableWindow(hwnd: *mut core::ffi::c_void, enable: i32) -> i32;
        fn SetForegroundWindow(hwnd: *mut core::ffi::c_void) -> i32;
    }
    unsafe {
        EnableWindow(owner, 1);
        SetForegroundWindow(owner);
    }
}
