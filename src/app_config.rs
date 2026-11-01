//! 编译期配置：`launcher.json` 在编译时通过 include_str! 嵌入二进制，
//! 运行时不可更改。修改配置后重新编译即生效。
//!
//! 字段说明：
//! - `app_name`: 应用名称（窗口标题、界面标题）
//! - `python_version`: 本机没有 Python 时自动部署的 embed 版 Python 版本号
//!   （如 "3.10.11"，用于拼接镜像/python.org 的 embed zip 下载地址）
//! - `python_dist_dir`: 内嵌 Python 发行版目录（相对启动器 exe 所在目录），
//!   为空字符串则跳过内嵌查找、直接回退 PATH 上的 python
//! - `install_packages`: 要安装/更新的主程序 pip 包名数组（纯名称，不含版本约束）
//! - `uninstall_packages`: 安装前统一卸载的旧包名数组（历史包名等，可为空数组）
//! - `entry_point`: 主程序入口点（`python -m <entry_point>` 的模块路径）
//! - `launch_command`: 启动主程序命令模板（必填），支持占位符：
//!   `{python}` 解释器路径、`{entry_point}` 入口点、
//!   `{extra_args}` 透传给主程序的额外参数

use serde_json::Value;
use std::sync::OnceLock;

use crate::i18n::{fill, strings};

const CONFIG_JSON: &str = include_str!("../launcher.json");

#[derive(Debug)]
pub struct AppConfig {
    pub app_name: String,
    pub python_version: String,
    pub python_dist_dir: String,
    pub install_packages: Vec<String>,
    pub uninstall_packages: Vec<String>,
    pub entry_point: String,
    pub launch_command: String,
}

static CONFIG: OnceLock<AppConfig> = OnceLock::new();

pub fn get() -> &'static AppConfig {
    CONFIG.get_or_init(|| {
        let v: Value = serde_json::from_str(CONFIG_JSON)
            .unwrap_or_else(|e| panic!("{}", fill(strings().config_parse_fail, &[&e.to_string()])));
        AppConfig {
            app_name: v["app_name"].as_str().unwrap_or("App").to_string(),
            python_version: v["python_version"].as_str().unwrap_or("3.10.11").to_string(),
            python_dist_dir: v["python_dist_dir"].as_str().unwrap_or("").to_string(),
            install_packages: string_array(&v, "install_packages"),
            uninstall_packages: string_array(&v, "uninstall_packages"),
            entry_point: required(&v, "entry_point"),
            launch_command: required(&v, "launch_command"),
        }
    })
}

/// 字符串数组字段：缺失或为空数组时 panic（install_packages 必须非空）
fn string_array(v: &Value, key: &str) -> Vec<String> {
    let arr: Vec<String> = v[key]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    if arr.is_empty() {
        panic!("{}", fill(strings().config_missing_field, &[key]));
    }
    arr
}

/// 必填字符串字段：缺失、非字符串或为空时立即 panic
fn required(v: &Value, key: &str) -> String {
    let s = v[key].as_str().unwrap_or("");
    if s.is_empty() {
        panic!("{}", fill(strings().config_missing_field, &[key]));
    }
    s.to_string()
}
