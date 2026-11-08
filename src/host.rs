//! 交互抽象：lib 侧不依赖任何 UI 框架，由各 bin 在启动时注册自己的实现。
//!
//! 公共流程（安装、更新、启动）在需要与用户交互时会调用这里的钩子：
//! - 首次运行询问所在地区（决定 PyPI 与 Python 发行版下载源）
//! - 本机没有 Python 时确认是否就地部署 embed 版
//! - 请求退出主循环（GUI 关事件循环；CLI 无此概念）
//! - 展示崩溃报告（GUI 弹窗；CLI 打印到 stderr）
//!
//! 这样 `lmm_launcher` 库保持与 Slint 无关，CLI 版二进制不必链接 GUI 依赖。

use std::sync::OnceLock;

use crate::settings::Region;

/// 宿主交互钩子。实现方必须线程安全：安装流程在后台线程调用这些方法。
pub trait Host: Send + Sync {
    /// 首次运行询问地区。GUI 弹模态窗口，CLI 走 stdin。
    fn ask_region(&self) -> Region;

    /// 询问是否同意某项操作（当前仅用于 embed Python 部署确认）。
    ///
    /// :param text: 正文。
    /// :param title: 标题。
    /// :return: 用户是否同意。
    fn confirm(&self, text: &str, title: &str) -> bool;

    /// 提示一条信息（无分支，仅告知）。
    ///
    /// 用于"用户拒绝部署后程序即将退出"这类场景：不给提示的话窗口会无声消失，
    /// 用户会当成崩溃。默认实现不做任何事，由各 bin 覆盖。
    ///
    /// :param text: 正文。
    /// :param title: 标题。
    fn notify(&self, text: &str, title: &str) {
        let _ = (text, title);
    }

    /// 请求退出主消息循环。可从任意线程调用。
    fn request_quit(&self);

    /// 展示崩溃报告（主程序异常退出）。
    ///
    /// :param title: 窗口/段落标题。
    /// :param hint: 说明文案。
    /// :param tail: 主程序输出尾部。
    fn crash(&self, title: &str, hint: &str, tail: &str);

    /// 致命错误后的反馈指引。GUI 版提示"附启动器窗口截图"，CLI 版没有窗口，
    /// 需要换成适合终端的说法。
    ///
    /// :param default: 默认文案（GUI 版本）。
    /// :return: 实际展示的文案。
    fn feedback_hint(&self, default: &str) -> String {
        default.to_string()
    }
}

static HOST: OnceLock<Box<dyn Host>> = OnceLock::new();

/// 注册当前 bin 的宿主实现。只允许注册一次，重复调用被忽略。
pub fn install(h: Box<dyn Host>) {
    let _ = HOST.set(h);
}

/// 取得已注册的宿主实现。
///
/// :return: 宿主钩子。
/// :raises panic: 未注册就调用时（bin 必须在处理任何参数前注册）。
pub fn get() -> &'static dyn Host {
    HOST.get()
        .expect("Host 未注册：每个 bin 都必须在启动时调用 host::install")
        .as_ref()
}
