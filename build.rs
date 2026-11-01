use std::path::{Path, PathBuf};

fn main() {
    // 配置来源：环境变量 LMM_LAUNCHER_JSON 指向源码树之外的任意 launcher.json
    // （集成方构建方式）；未设置时回退源码根 launcher.json，缺失则从示例生成，
    // 保证仓库开箱即可编译（占位配置，集成方须替换成自己的值）。
    let config_path: PathBuf = match std::env::var("LMM_LAUNCHER_JSON") {
        Ok(p) if !p.is_empty() => PathBuf::from(p),
        _ => {
            if !Path::new("launcher.json").exists() {
                std::fs::copy("launcher.example.json", "launcher.json")
                    .expect("launcher.json 缺失，且无法从 launcher.example.json 生成");
            }
            PathBuf::from("launcher.json")
        }
    };
    let config_path = std::fs::canonicalize(&config_path).expect("launcher.json 路径无法解析");
    // logoPath / icon 相对配置文件所在目录解析（集成方的资源跟着配置走）
    let config_dir = config_path.parent().unwrap().to_path_buf();
    println!("cargo:rerun-if-env-changed=LMM_LAUNCHER_JSON");
    println!("cargo:rerun-if-changed={}", config_path.display());

    slint_build::compile("ui/app.slint").unwrap();

    // 把配置文件绝对路径传给代码：include_str!(env!("LMM_LAUNCHER_CONFIG"))
    println!("cargo:rustc-env=LMM_LAUNCHER_CONFIG={}", config_path.display());

    let config = std::fs::read_to_string(&config_path).expect("读取 launcher.json 失败");
    let v: serde_json::Value = serde_json::from_str(&config).expect("launcher.json 解析失败");

    // splash logo（可选 logoPath）：为空或文件不存在时指向 OUT_DIR 空文件，
    // 运行时按图片宽度为 0 判定隐藏。
    // 注意：只要打印过一次 rerun-if-changed，cargo 就只跟踪这里列出的文件，
    // 所以配置文件必须显式声明（上方）
    let empty_logo = || {
        let out_dir = std::env::var("OUT_DIR").unwrap();
        let p = Path::new(&out_dir).join("empty_logo.bin");
        std::fs::write(&p, b"").unwrap();
        p.to_string_lossy().into_owned()
    };
    let logo_rel = v["logoPath"].as_str().unwrap_or("").trim();
    let logo_path = if !logo_rel.is_empty() {
        let p = config_dir.join(logo_rel);
        if p.exists() {
            println!("cargo:rerun-if-changed={}", p.display());
            std::fs::canonicalize(&p)
                .expect("logoPath 无法解析")
                .to_string_lossy()
                .into_owned()
        } else {
            empty_logo()
        }
    } else {
        empty_logo()
    };
    println!("cargo:rustc-env=LMM_LOGO_PATH={logo_path}");

    // 嵌入 exe 图标 + manifest（Windows）。
    // 注意只能调用一次 compile()——winresource 每次都会生成 VERSION 资源，
    // 调两次会产生重复资源导致 LNK1123。
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        let icon_rel = v["icon"].as_str().unwrap_or("").trim();
        if !icon_rel.is_empty() {
            let icon = config_dir.join(icon_rel);
            if icon.exists() {
                println!("cargo:rerun-if-changed={}", icon.display());
                res.set_icon(&icon.to_string_lossy());
            }
        }
        // manifest：common controls 6.0（原生控件主题化）+ PerMonitorV2 DPI
        println!("cargo:rerun-if-changed=app.manifest");
        let manifest = std::fs::read_to_string("app.manifest").expect("读取 app.manifest 失败");
        res.set_manifest(&manifest);
        res.compile().expect("嵌入图标/manifest 失败");
    }
}
