//! 运行时配置（区别于编译期 launcher.json）：exe 旁的 `launcher.conf.json`。
//!
//! 目前只有一项：`region`（"cn" | "intl"），决定 PyPI 镜像与 Python
//! 发行版下载源。首次启动（配置文件不存在）时通过 [`crate::host::Host::ask_region`]
//! 询问用户（GUI 弹模态窗口，CLI 走 stdin），选择后持久化，此后不再询问。

use std::path::PathBuf;
use std::sync::OnceLock;

/// 用户所在地区，决定下载源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// 中国大陆：使用国内 PyPI 镜像与 Python 发行版镜像
    Cn,
    /// 其他地区：直连 PyPI 官方源与 python.org
    Intl,
}

impl Region {
    /// 序列化用的短名。
    pub fn as_str(self) -> &'static str {
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

/// 加载地区配置；没有有效配置时询问用户并持久化。
///
/// 询问动作委托给 [`crate::host::Host::ask_region`]，因此 GUI / CLI 可各自实现。
pub fn ensure() -> Region {
    if let Some(r) = REGION.get() {
        return *r;
    }
    let region = load().unwrap_or_else(|| crate::host::get().ask_region());
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
