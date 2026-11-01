//! 界面与流程文案 i18n：跟随系统语言（中文简体 / English），二进制内固化两套文案。
//! 模板中的 `{}` 为运行时占位符，调用方用 `String::replace("{}", ...)` 填充
//! （多个占位符按出现顺序依次 replace）。

#[derive(Clone, Copy, PartialEq)]
pub enum Lang {
    ZhHans,
    En,
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn GetUserDefaultUILanguage() -> u16;
}

#[cfg(windows)]
fn detect_lang() -> Lang {
    let id = unsafe { GetUserDefaultUILanguage() };
    if id & 0x3ff == 0x04 {
        Lang::ZhHans
    } else {
        Lang::En
    }
}

#[cfg(not(windows))]
fn detect_lang() -> Lang {
    match std::env::var("LANG").unwrap_or_default().to_lowercase().starts_with("zh") {
        true => Lang::ZhHans,
        false => Lang::En,
    }
}

#[allow(dead_code)]
pub struct Strings {
    // ---- 主窗口固定文案 ----
    pub ready: &'static str,
    pub details: &'static str,
    pub collapse_details: &'static str,
    /// 退出确认消息框：标题 / 正文（正文含 `{}` 占位符，运行时替换为应用名）
    pub confirm_exit_title: &'static str,
    pub confirm_exit: &'static str,

    // ---- 地区选择窗口 ----
    pub region_instruction: &'static str,
    pub region_body: &'static str,
    pub region_cn: &'static str,
    pub region_intl: &'static str,

    // ---- 主流程 ----
    pub initializing: &'static str,
    pub python_not_found: &'static str,
    pub finding_mirror: &'static str,
    /// `{}` = 镜像地址
    pub testing_server: &'static str,
    pub server_ok: &'static str,
    pub no_pip_server: &'static str,
    pub no_pip_server_err: &'static str,

    /// pip 进度：`{}` 依次为 包名, 已完成, 总数
    pub downloading_pkg: &'static str,
    /// `{}` = 包数
    pub installing_pkgs: &'static str,
    pub install_done: &'static str,
    pub analyzing_deps: &'static str,
    /// `{}` = 错误
    pub analyze_fail_log: &'static str,
    pub installing: &'static str,
    /// `{}` = 返回码
    pub install_fail: &'static str,
    pub pip_start_fail: &'static str,
    pub upgrade_fail: &'static str,
    /// `{}` = 包名, 错误
    pub uninstall_fail: &'static str,
    /// `{}` = 包名, 错误（卸载失败但继续安装的日志）
    pub uninstall_fail_continue: &'static str,
    /// `{}` = 错误
    pub versions_fail: &'static str,
    pub parse_versions_fail: &'static str,
    /// `{}` = 错误（dry-run 硬失败）
    pub analyze_fail: &'static str,

    pub uninstalling_existing: &'static str,
    /// `{}` = 包名, 版本
    pub installing_version_log: &'static str,
    /// `{}` = 路径
    pub zip_not_found: &'static str,
    /// `{}` = 路径
    pub install_from_zip: &'static str,
    pub extracting_zip: &'static str,
    /// `{}` = 包名
    pub installing_pkg_log: &'static str,
    /// `{}` = 路径
    pub pkg_not_found: &'static str,
    /// `{}` = 路径
    pub not_pkg_file: &'static str,
    /// `{}` = 路径
    pub install_from_pkg: &'static str,

    pub upgrading_pip: &'static str,
    pub checking_update: &'static str,
    /// `{}` = 包名
    pub no_versions: &'static str,
    pub not_installed: &'static str,
    /// `{}` = 旧版本, 新版本
    pub new_version: &'static str,
    /// `{}` = 应用名, 版本
    pub up_to_date: &'static str,

    /// `{}` = 应用名
    pub waiting_start: &'static str,
    pub launch_cmd_empty: &'static str,
    /// `{}` = 错误
    pub spawn_fail: &'static str,
    /// `{}` = 错误
    pub wait_fail: &'static str,
    /// `{}` = 应用名
    pub app_exited: &'static str,
    /// `{}` = 应用名（崩溃窗口标题）
    pub crash_title: &'static str,
    /// `{}` = 应用名, 退出码（崩溃窗口提示行）
    pub crash_hint: &'static str,
    /// `{}` = 退出码
    pub crash_exit_log: &'static str,
    pub ok_btn: &'static str,

    pub installing_zip_status: &'static str,
    pub installing_pkg_status: &'static str,
    pub installing_version_status: &'static str,
    pub install_update_status: &'static str,
    /// `{}` = 错误
    pub fatal_error: &'static str,
    pub feedback_hint: &'static str,
    /// `{}` = 错误
    pub error_prefix: &'static str,

    // ---- 恢复模式（流程保留，UI 入口未接） ----
    pub fetching_versions: &'static str,
    pub no_versions_net: &'static str,
    /// `{}` = 版本
    pub current_version: &'static str,
    pub versions_listed: &'static str,
    /// `{}` = 版本
    pub installing_version_status2: &'static str,
    /// `{}` = 应用名
    pub installed_restart: &'static str,

    // ---- Python 部署（deploy.rs） ----
    /// `{}` = 应用名, 目录（部署询问框正文）
    pub deploy_prompt: &'static str,
    pub deploy_prompt_title: &'static str,
    /// `{}` = 版本
    pub downloading_python: &'static str,
    /// `{}` = 错误
    pub download_fail_log: &'static str,
    /// `{}` = 错误
    pub download_python_fail: &'static str,
    pub extracting_python: &'static str,
    pub configuring_pip: &'static str,
    /// `{}` = 版本
    pub python_ready: &'static str,
    /// `{}` = URL
    pub downloading_url: &'static str,
    /// `{}` = 百分比
    pub downloading_pct: &'static str,
    /// `{}` = 错误
    pub request_fail: &'static str,
    /// `{}` = 错误
    pub create_file_fail: &'static str,
    /// `{}` = 错误
    pub download_interrupted: &'static str,
    /// `{}` = 错误
    pub write_fail: &'static str,
    /// `{}` = 已下载, 总字节
    pub download_incomplete: &'static str,
    /// `{}` = 错误
    pub open_zip_fail: &'static str,
    /// `{}` = 错误
    pub invalid_zip: &'static str,
    /// `{}` = 错误
    pub create_dir_fail: &'static str,
    /// `{}` = 错误
    pub read_zip_fail: &'static str,
    pub no_pth_file: &'static str,
    pub fetching_pip_index: &'static str,
    /// `{}` = 错误
    pub pip_index_fail: &'static str,
    /// `{}` = 错误
    pub read_index_fail: &'static str,
    pub no_pip_wheel: &'static str,
    /// `{}` = 文件名
    pub downloading_file: &'static str,
    /// `{}` = 错误
    pub temp_dir_fail: &'static str,
    /// `{}` = 路径
    pub extracting: &'static str,
    pub installing_pip_status: &'static str,
    pub installing_pip_log: &'static str,
    /// `{}` = 错误
    pub start_python_fail: &'static str,
    /// `{}` = 返回码
    pub install_pip_fail: &'static str,
    pub installing_setuptools: &'static str,
    pub installing_setuptools_log: &'static str,
    /// `{}` = 返回码
    pub install_setuptools_fail: &'static str,
    /// `{}` = 错误
    pub exe_dir_fail: &'static str,
    pub user_cancelled_deploy: &'static str,

    // ---- Job Object（job.rs） ----
    pub job_create_fail: &'static str,
    pub job_limit_fail: &'static str,
    pub job_assign_fail: &'static str,

    // ---- 其他日志 ----
    /// `{}` = 路径
    pub python_path_log: &'static str,
    /// `{}` = 错误
    pub set_cwd_fail: &'static str,
    /// `{}` = 错误
    pub crash_window_fail: &'static str,
    pub launcher_started: &'static str,
    /// `{}` = 版本
    pub launcher_version: &'static str,
    /// `{}` = 版本
    pub python_exists: &'static str,

    // ---- 启动期致命错误（panic / 参数解析） ----
    /// `{}` = 错误
    pub config_parse_fail: &'static str,
    /// `{}` = 字段名
    pub config_missing_field: &'static str,
    pub region_not_init: &'static str,
    /// `{}` = 参数
    pub unsupported_file_type: &'static str,
    /// `{}` = 命令
    pub running_cmd: &'static str,
    pub window_create_fail: &'static str,
    pub window_show_fail: &'static str,
    pub event_loop_fail: &'static str,
}

const ZH: Strings = Strings {
    ready: "就绪",
    details: "详细信息",
    collapse_details: "收起详情",
    confirm_exit_title: "提示",
    confirm_exit: "正在安装或升级「{}」，确定要退出吗？",

    region_instruction: "为了提升网络下载体验，请选择你所在的地区。",
    region_body: "这将决定软件下载源（PyPI 镜像等），之后可在启动器旁的 launcher.conf.json 中修改。",
    region_cn: "中国大陆",
    region_intl: "其他地区",

    initializing: "初始化...",
    python_not_found: "未找到 Python 解释器，准备部署...",
    finding_mirror: "寻找最快的 PyPI 镜像源...",
    testing_server: "  正在测试: {}",
    server_ok: "  找到可用的pip服务器: {}",
    no_pip_server: "所有pip服务器都不可用",
    no_pip_server_err: "没有找到可用的pip服务器，请检查网络连接。",

    downloading_pkg: "正在下载 {} ({}/{})",
    installing_pkgs: "正在安装共 {} 个包",
    install_done: "安装完成",
    analyzing_deps: "正在解析待安装包...",
    analyze_fail_log: "  依赖分析失败（不影响安装）: {}",
    installing: "正在安装...",
    install_fail: "安装失败，返回码: {}",
    pip_start_fail: "无法启动 pip: {}",
    upgrade_fail: "更新 pip 失败，返回码: {}",
    uninstall_fail: "卸载 {} 失败，返回码: {}",
    uninstall_fail_continue: "  卸载 {} 失败: {}，继续安装",
    versions_fail: "获取版本列表失败: {}",
    parse_versions_fail: "解析版本列表失败: {}",
    analyze_fail: "分析依赖失败: {}",

    uninstalling_existing: "卸载现有主程序...",
    installing_version_log: "安装 {} v{}",
    zip_not_found: "zip文件不存在: {}",
    install_from_zip: "从zip文件安装: {}",
    extracting_zip: "解压zip文件...",
    installing_pkg_log: "安装 {} 包...",
    pkg_not_found: "包文件不存在: {}",
    not_pkg_file: "文件不是 .whl 或 .tar.gz 格式: {}",
    install_from_pkg: "从包文件安装: {}",

    upgrading_pip: "更新 pip",
    checking_update: "检查主程序更新",
    no_versions: "没有可用的 {} 版本",
    not_installed: "未安装主程序，正在安装...",
    new_version: "发现新版本: {} -> {}",
    up_to_date: "{} 已是最新版本: {}",

    waiting_start: "正在等待{}启动……",
    launch_cmd_empty: "launch_command 配置为空，无法启动主程序",
    spawn_fail: "无法启动主程序: {}",
    wait_fail: "等待主程序退出失败: {}",
    app_exited: "{} 已退出。",
    crash_title: "{} 似乎崩溃了",
    crash_hint: "「{}」似乎崩溃了（退出码 {}）",
    crash_exit_log: "主程序异常退出，退出码 {}",
    ok_btn: "确定",

    installing_zip_status: "安装补丁（zip）...",
    installing_pkg_status: "安装补丁（包文件）...",
    installing_version_status: "安装指定版本...",
    install_update_status: "安装与更新主程序...",
    fatal_error: "发生致命错误: {}",
    feedback_hint: "如需反馈，压缩本目录下的 logs 文件夹并附上启动器窗口截图发送给开发者",
    error_prefix: "发生错误: {}",

    fetching_versions: "正在获取可用版本列表...",
    no_versions_net: "没有找到可用的版本，请检查网络连接。",
    current_version: "当前版本: {}",
    versions_listed: "可用版本已列出，请在下方选择要安装的版本。",
    installing_version_status2: "正在安装版本 {}...",
    installed_restart: "安装成功，请重启 {}",

    deploy_prompt: "尚未安装「{}」。是否在下面的文件夹内下载并安装？\n{}\n我们建议你新建一个空文件夹，移动本程序到这个文件夹下再继续安装。",
    deploy_prompt_title: "提示",
    downloading_python: "正在下载 Python {}（embed）...",
    download_fail_log: "下载失败: {}",
    download_python_fail: "下载 Python 失败: {}",
    extracting_python: "正在解压 Python...",
    configuring_pip: "正在配置 pip...",
    python_ready: "Python {} 部署完成",
    downloading_url: "下载 {}",
    downloading_pct: "正在下载（{}%）",
    request_fail: "请求失败: {}",
    create_file_fail: "创建文件失败: {}",
    download_interrupted: "下载中断: {}",
    write_fail: "写入文件失败: {}",
    download_incomplete: "下载不完整: {}/{} 字节",
    open_zip_fail: "打开 zip 失败: {}",
    invalid_zip: "无效的 zip 文件: {}",
    create_dir_fail: "创建目录失败: {}",
    read_zip_fail: "读取 zip 失败: {}",
    no_pth_file: "未找到 python ._pth 配置文件，无法启用 site-packages",
    fetching_pip_index: "获取 pip 版本列表...",
    pip_index_fail: "获取 pip 索引失败: {}",
    read_index_fail: "读取 pip 索引失败: {}",
    no_pip_wheel: "未能在镜像源中找到 pip 的安装包",
    downloading_file: "下载 {}...",
    temp_dir_fail: "创建临时目录失败: {}",
    extracting: "解压 {}...",
    installing_pip_status: "正在安装 pip...",
    installing_pip_log: "安装 pip...",
    start_python_fail: "无法启动 Python: {}",
    install_pip_fail: "安装 pip 失败，返回码: {}",
    installing_setuptools: "正在安装 setuptools...",
    installing_setuptools_log: "安装 setuptools...",
    install_setuptools_fail: "安装 setuptools 失败，返回码: {}",
    exe_dir_fail: "无法获取程序位置: {}",
    user_cancelled_deploy: "用户取消了 Python 环境安装，启动器退出",

    job_create_fail: "创建 Job Object 失败，子进程将不受退出清理保护",
    job_limit_fail: "设置 Job Object 限制失败",
    job_assign_fail: "子进程挂入 Job Object 失败",

    python_path_log: "Python 路径: {}",
    set_cwd_fail: "无法设置工作目录: {}",
    crash_window_fail: "无法显示崩溃报告窗口: {}",
    launcher_started: "启动器已启动。",
    launcher_version: "启动器版本: {}",
    python_exists: "Python {} 已存在: {}",

    config_parse_fail: "launcher.json 编译期配置解析失败: {}",
    config_missing_field: "launcher.json 缺少必填配置项: {}",
    region_not_init: "region 未初始化（应先调用 settings::ensure）",
    unsupported_file_type: "不支持的文件类型: {}",
    running_cmd: "运行: {}",
    window_create_fail: "无法创建主窗口",
    window_show_fail: "无法显示主窗口",
    event_loop_fail: "Slint 事件循环错误",
};

const EN: Strings = Strings {
    ready: "Ready",
    details: "Details",
    collapse_details: "Hide Details",
    confirm_exit_title: "Confirm",
    confirm_exit: "\"{}\" is installing or upgrading. Are you sure you want to quit?",

    region_instruction: "To improve your download experience, please select your region.",
    region_body: "This decides the download sources (PyPI mirrors etc.). You can change it later in launcher.conf.json next to the launcher.",
    region_cn: "Mainland China",
    region_intl: "Other regions",

    initializing: "Initializing...",
    python_not_found: "No Python interpreter found. Preparing deployment...",
    finding_mirror: "Finding the fastest PyPI mirror...",
    testing_server: "  Testing: {}",
    server_ok: "  Usable pip server found: {}",
    no_pip_server: "No usable pip server",
    no_pip_server_err: "No usable pip server found. Please check your network connection.",

    downloading_pkg: "Downloading {} ({}/{})",
    installing_pkgs: "Installing {} packages",
    install_done: "Installation complete",
    analyzing_deps: "Analyzing packages to install...",
    analyze_fail_log: "  Dependency analysis failed (install continues): {}",
    installing: "Installing...",
    install_fail: "Install failed, exit code: {}",
    pip_start_fail: "Failed to start pip: {}",
    upgrade_fail: "Failed to upgrade pip, exit code: {}",
    uninstall_fail: "Failed to uninstall {}, exit code: {}",
    uninstall_fail_continue: "  Failed to uninstall {}: {}, continuing",
    versions_fail: "Failed to get version list: {}",
    parse_versions_fail: "Failed to parse version list: {}",
    analyze_fail: "Dependency analysis failed: {}",

    uninstalling_existing: "Uninstalling existing program...",
    installing_version_log: "Installing {} v{}",
    zip_not_found: "Zip file not found: {}",
    install_from_zip: "Installing from zip: {}",
    extracting_zip: "Extracting zip...",
    installing_pkg_log: "Installing {} package...",
    pkg_not_found: "Package file not found: {}",
    not_pkg_file: "Not a .whl or .tar.gz file: {}",
    install_from_pkg: "Installing from package: {}",

    upgrading_pip: "Upgrading pip",
    checking_update: "Checking for updates",
    no_versions: "No available versions of {}",
    not_installed: "Main program not installed. Installing...",
    new_version: "New version found: {} -> {}",
    up_to_date: "{} is up to date: {}",

    waiting_start: "Waiting for {} to start...",
    launch_cmd_empty: "launch_command is empty; cannot start the main program",
    spawn_fail: "Failed to start the main program: {}",
    wait_fail: "Failed while waiting for the main program: {}",
    app_exited: "{} has exited.",
    crash_title: "{} seems to have crashed",
    crash_hint: "\"{}\" seems to have crashed (exit code {})",
    crash_exit_log: "Main program exited abnormally, exit code {}",
    ok_btn: "OK",

    installing_zip_status: "Installing patch (zip)...",
    installing_pkg_status: "Installing patch (package file)...",
    installing_version_status: "Installing specific version...",
    install_update_status: "Installing/updating main program...",
    fatal_error: "Fatal error: {}",
    feedback_hint: "To report an issue, zip the logs folder in this directory and send it to the developer with a screenshot of the launcher window",
    error_prefix: "Error: {}",

    fetching_versions: "Fetching available version list...",
    no_versions_net: "No available versions found. Please check your network connection.",
    current_version: "Current version: {}",
    versions_listed: "Available versions listed. Please choose one below.",
    installing_version_status2: "Installing version {}...",
    installed_restart: "Installed. Please restart {}",

    deploy_prompt: "\"{}\" is not installed yet. Download and install it in the folder below?\n{}\nWe recommend creating a new empty folder and moving this program into it before continuing.",
    deploy_prompt_title: "Notice",
    downloading_python: "Downloading Python {} (embed)...",
    download_fail_log: "Download failed: {}",
    download_python_fail: "Failed to download Python: {}",
    extracting_python: "Extracting Python...",
    configuring_pip: "Configuring pip...",
    python_ready: "Python {} deployed",
    downloading_url: "Downloading {}",
    downloading_pct: "Downloading ({}%)",
    request_fail: "Request failed: {}",
    create_file_fail: "Failed to create file: {}",
    download_interrupted: "Download interrupted: {}",
    write_fail: "Failed to write file: {}",
    download_incomplete: "Incomplete download: {}/{} bytes",
    open_zip_fail: "Failed to open zip: {}",
    invalid_zip: "Invalid zip file: {}",
    create_dir_fail: "Failed to create directory: {}",
    read_zip_fail: "Failed to read zip: {}",
    no_pth_file: "python ._pth config not found; cannot enable site-packages",
    fetching_pip_index: "Fetching pip version list...",
    pip_index_fail: "Failed to fetch pip index: {}",
    read_index_fail: "Failed to read pip index: {}",
    no_pip_wheel: "pip wheel not found on the index",
    downloading_file: "Downloading {}...",
    temp_dir_fail: "Failed to create temp directory: {}",
    extracting: "Extracting {}...",
    installing_pip_status: "Installing pip...",
    installing_pip_log: "Install pip...",
    start_python_fail: "Failed to start Python: {}",
    install_pip_fail: "Failed to install pip, exit code: {}",
    installing_setuptools: "Installing setuptools...",
    installing_setuptools_log: "Install setuptools...",
    install_setuptools_fail: "Failed to install setuptools, exit code: {}",
    exe_dir_fail: "Failed to get program location: {}",
    user_cancelled_deploy: "User cancelled Python deployment; launcher exiting",

    job_create_fail: "Failed to create Job Object; child processes won't be cleaned up on exit",
    job_limit_fail: "Failed to set Job Object limits",
    job_assign_fail: "Failed to assign child process to Job Object",

    python_path_log: "Python path: {}",
    set_cwd_fail: "Failed to set working directory: {}",
    crash_window_fail: "Failed to show crash report window: {}",
    launcher_started: "Launcher started.",
    launcher_version: "Launcher version: {}",
    python_exists: "Python {} already exists: {}",

    config_parse_fail: "Failed to parse launcher.json: {}",
    config_missing_field: "launcher.json is missing required field: {}",
    region_not_init: "region not initialized (call settings::ensure first)",
    unsupported_file_type: "Unsupported file type: {}",
    running_cmd: "Running: {}",
    window_create_fail: "Failed to create the main window",
    window_show_fail: "Failed to show the main window",
    event_loop_fail: "Slint event loop error",
};

pub fn strings() -> &'static Strings {
    use std::sync::OnceLock;
    static STRINGS: OnceLock<Strings> = OnceLock::new();
    STRINGS.get_or_init(|| match detect_lang() {
        Lang::ZhHans => ZH,
        Lang::En => EN,
    })
}

/// 把模板中的 `{}` 按顺序替换为参数。
/// `fill(tmpl, &["a", "b"])` —— 占位符数量需与参数一致。
pub fn fill(tmpl: &str, args: &[&str]) -> String {
    let mut out = tmpl.to_string();
    for a in args {
        out = out.replacen("{}", a, 1);
    }
    out
}
