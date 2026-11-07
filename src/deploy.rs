//! Python 解释器缺失时的自动部署：
//! 1. 弹出原生消息框询问用户是否在配置所在目录下载安装（取消则终止）
//! 2. 按用户选择的地区（settings::Region）从镜像或 python.org 下载 embed 版
//!    Python（版本来自编译期配置 launcher.json 的 `python_version`），
//!    解压到 `python_path` 指向文件的父目录
//! 3. 修改 `pythonXY._pth` 启用 `import site`，使 site-packages 生效
//! 4. 从所选源下载 pip 的通用 wheel 并安装
//!
//! 部署目标由 `python_path` 决定（而不是按版本拼目录），因此部署完的路径与
//! 配置永远一致，`Pip::find_python()` 随后即可命中。

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use zip::ZipArchive;

use crate::i18n::{fill, strings};
use crate::pip::HideConsoleWindow;
use crate::settings::Region;
use crate::Ui;

/// 清华 TUNA 的 Python 官方发行版镜像（镜像 www.python.org/ftp/python）
const TUNA_PYTHON_MIRROR: &str = "https://mirrors.tuna.tsinghua.edu.cn/python";
/// 官方源
const PYTHON_OFFICIAL_FTP: &str = "https://www.python.org/ftp/python";

/// 配置 `python_path` 的绝对路径：相对路径按 exe 所在目录解析。
///
/// 返回值**不保证文件存在**——不存在时由 [`ensure_python`] 把 embed 版
/// 下载解压到该路径的父目录，因此路径本身就是部署目标。
pub fn configured_python_path() -> Option<PathBuf> {
    let raw = &crate::app_config::get().python_path;
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        return Some(path);
    }
    Some(crate::settings::exe_dir().ok()?.join(path))
}

/// embed zip 的候选下载地址（按地区，优先级从高到低）
fn embed_zip_urls(version: &str) -> Vec<String> {
    let file = format!("python-{version}-embed-amd64.zip");
    match crate::settings::get() {
        Region::Cn => vec![
            format!("{TUNA_PYTHON_MIRROR}/{version}/{file}"),
            format!("{PYTHON_OFFICIAL_FTP}/{version}/{file}"),
        ],
        Region::Intl => vec![format!("{PYTHON_OFFICIAL_FTP}/{version}/{file}")],
    }
}

/// 消息框询问用户是否在本目录部署 Python
#[cfg(windows)]
fn prompt_user(app_name: &str, dir: &Path) -> bool {
    let s = strings();
    let text = fill(s.deploy_prompt, &[app_name, &dir.display().to_string()]);
    // 交互形式交给 bin：GUI 弹原生消息框（挂主窗口为属主，否则会被主窗口遮挡），
    // CLI 在终端询问。这样 lib 不依赖任何窗口框架。
    crate::host::get().confirm(&text, s.deploy_prompt_title)
}

/// 确保配置里的解释器可用：不存在时询问并自动部署 embed 版，返回解释器路径。
///
/// 部署目标是 `python_path` 的父目录，因此下载完成后的路径与配置永远一致，
/// 不需要"先按版本拼目录、再比对配置"。
pub fn ensure_python(ui: &Ui) -> Result<PathBuf, String> {
    let config = crate::app_config::get();
    let python_exe = configured_python_path()
        .ok_or_else(|| strings().python_not_found.to_string())?;
    let target_dir = python_exe
        .parent()
        .ok_or_else(|| strings().python_not_found.to_string())?;
    #[cfg(windows)]
    {
        if !prompt_user(&config.app_name, target_dir) {
            crate::logging::info(strings().user_cancelled_deploy);
            std::process::exit(0);
        }
    }
    deploy_embedded_python(ui, &python_exe, &config.python_version)
}

/// 下载 embed 版并解压到 `python_exe` 的父目录。
fn deploy_embedded_python(ui: &Ui, python_exe: &Path, version: &str) -> Result<PathBuf, String> {
    if python_exe.exists() {
        ui.log(&fill(strings().python_exists, &[version, &python_exe.display().to_string()]));
        return Ok(python_exe.to_path_buf());
    }
    let target = python_exe
        .parent()
        .ok_or_else(|| strings().python_not_found.to_string())?;

    // 下载 embed zip：按地区选择源，失败时依次尝试候选地址
    ui.status(&fill(strings().downloading_python, &[version]));
    let zip_path = target.join(format!("python-{version}-embed-amd64.zip"));
    let mut last_err: Option<String> = None;
    for url in &embed_zip_urls(version) {
        match download(ui, url, &zip_path, 0.0, 7.0) {
            Ok(()) => {
                last_err = None;
                break;
            }
            Err(e) => {
                ui.log(&fill(strings().download_fail_log, &[&e]));
                last_err = Some(e);
            }
        }
    }
    if let Some(e) = last_err {
        return Err(fill(strings().download_python_fail, &[&e]));
    }

    ui.status(strings().extracting_python);
    extract_zip(&zip_path, target)?;
    ui.progress(8.0);

    enable_import_site(target)?;

    ui.status(strings().configuring_pip);
    install_pip(ui, python_exe)?;
    ui.progress(10.0);

    // 清理下载的 zip
    let _ = std::fs::remove_file(&zip_path);
    crate::logging::info(&fill(strings().python_ready, &[version, &python_exe.display().to_string()]));
    ui.log(&fill(strings().python_ready, &[version]));
    Ok(python_exe.to_path_buf())
}

/// 下载文件到 dest，进度上报到 UI（base ~ base+span 的进度区间）
fn download(ui: &Ui, url: &str, dest: &Path, base: f32, span: f32) -> Result<(), String> {
    ui.log(&fill(strings().downloading_url, &[url]));
    let resp = ureq::get(url)
        .call()
        .map_err(|e| fill(strings().request_fail, &[&e.to_string()]))?;
    let total: u64 = resp
        .header("Content-Length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut reader = resp.into_reader();
    let file = std::fs::File::create(dest)
        .map_err(|e| fill(strings().create_file_fail, &[&e.to_string()]))?;
    let mut writer = std::io::BufWriter::new(file);
    let mut buf = [0u8; 64 * 1024];
    let mut got: u64 = 0;
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| fill(strings().download_interrupted, &[&e.to_string()]))?;
        if n == 0 {
            break;
        }
        writer
            .write_all(&buf[..n])
            .map_err(|e| fill(strings().write_fail, &[&e.to_string()]))?;
        got += n as u64;
        if total > 0 {
            let frac = got as f32 / total as f32;
            ui.status(&fill(strings().downloading_pct, &[&((frac * 100.0) as u32).to_string()]));
            ui.progress(base + span * frac);
        }
    }
    if total > 0 && got != total {
        return Err(fill(strings().download_incomplete, &[&got.to_string(), &total.to_string()]));
    }
    Ok(())
}

fn extract_zip(zip_path: &Path, target: &Path) -> Result<(), String> {
    let s = strings();
    let file =
        std::fs::File::open(zip_path).map_err(|e| fill(s.open_zip_fail, &[&e.to_string()]))?;
    let mut archive =
        ZipArchive::new(file).map_err(|e| fill(s.invalid_zip, &[&e.to_string()]))?;
    std::fs::create_dir_all(target).map_err(|e| fill(s.create_dir_fail, &[&e.to_string()]))?;
    for i in 0..archive.len() {
        let mut entry =
            archive.by_index(i).map_err(|e| fill(s.read_zip_fail, &[&e.to_string()]))?;
        let Some(name) = entry.enclosed_name() else { continue };
        let out_path = target.join(name);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)
                .map_err(|e| fill(s.create_dir_fail, &[&e.to_string()]))?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| fill(s.create_dir_fail, &[&e.to_string()]))?;
            }
            let mut out = std::fs::File::create(&out_path)
                .map_err(|e| fill(s.create_file_fail, &[&e.to_string()]))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|e| fill(s.write_fail, &[&e.to_string()]))?;
        }
    }
    Ok(())
}

/// embed 版 Python 默认禁用 site（`#import site`），不启用则 site-packages 不可用。
/// 找到 pythonXY._pth 并取消注释 `import site`。
fn enable_import_site(dir: &Path) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| fill(strings().open_zip_fail, &[&e.to_string()]))? {
        let entry = entry.map_err(|e| fill(strings().open_zip_fail, &[&e.to_string()]))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("python") && name.ends_with("._pth") {
            let path = entry.path();
            let content = std::fs::read_to_string(&path)
                .map_err(|e| fill(strings().open_zip_fail, &[&e.to_string()]))?;
            if content.contains("#import site") {
                std::fs::write(&path, content.replace("#import site", "import site"))
                    .map_err(|e| fill(strings().write_fail, &[&e.to_string()]))?;
            }
            return Ok(());
        }
    }
    Err(strings().no_pth_file.to_string())
}

/// embed 版 Python 不带 pip：从所选源的 simple 索引下载 pip 通用 wheel。
/// 注意 embed 版的 `._pth` 机制会忽略 PYTHONPATH 等所有环境变量，所以不能走
/// PYTHONPATH 注入，而是把 wheel 直接解压进 `Lib/site-packages` 后用
/// `python -m pip install` 自举安装（`import site` 已由 enable_import_site 启用）。
fn install_pip(ui: &Ui, python_exe: &Path) -> Result<(), String> {
    let s = strings();
    let server = crate::get_working_pip_server(ui)
        .ok_or_else(|| s.no_pip_server_err.to_string())?;

    ui.log(s.fetching_pip_index);
    let index_url = format!("{}/pip/", server.trim_end_matches('/'));
    let page = ureq::get(&index_url)
        .call()
        .map_err(|e| fill(s.pip_index_fail, &[&e.to_string()]))?
        .into_string()
        .map_err(|e| fill(s.read_index_fail, &[&e.to_string()]))?;
    let (href, wheel_name) = pick_pip_wheel(&page).ok_or_else(|| s.no_pip_wheel.to_string())?;
    let wheel_url = resolve_url(&index_url, &href);
    ui.log(&fill(s.downloading_file, &[&wheel_name]));

    let temp_dir = tempfile::tempdir().map_err(|e| fill(s.temp_dir_fail, &[&e.to_string()]))?;
    let wheel_path = temp_dir.path().join(&wheel_name);
    download(ui, &wheel_url, &wheel_path, 8.0, 1.0)?;

    ui.log(&fill(s.extracting, &[&wheel_name]));
    let site_packages = python_exe
        .parent()
        .ok_or_else(|| s.no_pip_wheel.to_string())?
        .join("Lib")
        .join("site-packages");
    std::fs::create_dir_all(&site_packages)
        .map_err(|e| fill(s.create_dir_fail, &[&e.to_string()]))?;
    extract_zip(&wheel_path, &site_packages)?;
    ui.progress(9.0);

    ui.status(s.installing_pip_status);
    ui.log(s.installing_pip_log);
    let out = std::process::Command::new(python_exe)
        .args(["-m", "pip", "install", "--no-index", "--upgrade"])
        .arg(&wheel_path)
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUNBUFFERED", "1")
        .hide_console_window()
        .output()
        .map_err(|e| fill(s.start_python_fail, &[&e.to_string()]))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        ui.log(&format!("  {line}"));
    }
    if !out.status.success() {
        return Err(fill(
            s.install_pip_fail,
            &[&out.status.code().unwrap_or(-1).to_string()],
        ));
    }

    // embed 版没有 setuptools：依赖中若含 sdist 包，pip 构建时会报
    // BackendUnavailable: Cannot import 'setuptools.build_meta'，这里补装
    ui.status(s.installing_setuptools);
    ui.log(s.installing_setuptools_log);
    let trusted = crate::pip::TRUSTED_HOSTS.join(" ");
    let out = std::process::Command::new(python_exe)
        .args(["-m", "pip", "install", "--upgrade", "setuptools"])
        .args(["--index-url", &server])
        .args(["--trusted-host", &trusted])
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUNBUFFERED", "1")
        .hide_console_window()
        .output()
        .map_err(|e| fill(s.start_python_fail, &[&e.to_string()]))?;
    if !out.status.success() {
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        for line in text.lines().filter(|l| !l.trim().is_empty()).take(10) {
            ui.log(&format!("  {line}"));
        }
        return Err(fill(
            s.install_setuptools_fail,
            &[&out.status.code().unwrap_or(-1).to_string()],
        ));
    }
    Ok(())
}

/// 在 simple 索引页面中选取最新的 pip 通用 wheel（py3-none-any，跳过 rc/beta 等预发布版本）
fn pick_pip_wheel(page: &str) -> Option<(String, String)> {
    let mut found: Option<(String, String)> = None;
    for line in page.lines() {
        let Some(pos) = line.find("href=\"") else { continue };
        let rest = &line[pos + 6..];
        let Some(end) = rest.find('"') else { continue };
        let href = &rest[..end];
        let tail = href.rsplit('/').next().unwrap_or("");
        let name = tail.split('#').next().unwrap_or("");
        if !(name.starts_with("pip-") && name.ends_with("-py3-none-any.whl")) {
            continue;
        }
        let ver = name
            .trim_start_matches("pip-")
            .trim_end_matches("-py3-none-any.whl");
        // 仅接受 x.y.z 形式的正式版本
        if !ver.is_empty() && ver.chars().all(|c| c.is_ascii_digit() || c == '.') {
            found = Some((href.to_string(), name.to_string()));
        }
    }
    found
}

/// 相对 URL 解析（处理以 / 开头与 ../ 两种形式）
fn resolve_url(base: &str, href: &str) -> String {
    let href = href.split('#').next().unwrap_or(href);
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_string();
    }
    if href.starts_with('/') {
        // 协议相对：取 origin + 路径
        if let Some(rest) = base.strip_prefix("https://") {
            let origin = rest.split('/').next().unwrap_or("");
            return format!("https://{origin}{href}");
        }
        if let Some(rest) = base.strip_prefix("http://") {
            let origin = rest.split('/').next().unwrap_or("");
            return format!("http://{origin}{href}");
        }
    }
    let base_dir = match base.rfind('/') {
        Some(i) => &base[..=i],
        None => "/",
    };
    let mut path = format!("{base_dir}{href}");
    loop {
        let Some(i) = path.find("../") else { break };
        let before = &path[..i];
        let prev = before[..before.len().saturating_sub(1)]
            .rfind('/')
            .map(|p| p + 1)
            .unwrap_or(0);
        path = format!("{}{}", &path[..prev], &path[i + 3..]);
    }
    path
}
