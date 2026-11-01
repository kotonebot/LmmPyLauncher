fn main() {
    // launcher.json 是集成方私有配置（不进 git）；仓库初次 clone 时从示例生成，
    // 让项目开箱即可编译（占位配置，集成方必须替换成自己的值）
    if !std::path::Path::new("launcher.json").exists() {
        std::fs::copy("launcher.example.json", "launcher.json")
            .expect("launcher.json 缺失，且无法从 launcher.example.json 生成");
    }

    slint_build::compile("ui/app.slint").unwrap();

    // 编译期配置（launcher.json）：splash logo（可选 logoPath）。
    // 通过环境变量把 logo 文件路径传给 include_bytes! 嵌入二进制；
    // 未配置时指向 OUT_DIR 下的空文件，运行时按图片宽度为 0 判定隐藏。
    // 注意：只要打印过一次 rerun-if-changed，cargo 就只跟踪这里列出的文件，
    // launcher.json 必须显式声明，否则改配置不会重新嵌入
    println!("cargo:rerun-if-changed=launcher.json");
    let logo = std::fs::read_to_string("launcher.json")
        .ok()
        .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
        .and_then(|v| v["logoPath"].as_str().map(String::from))
        .unwrap_or_default();
    let logo_path = if !logo.is_empty() && std::path::Path::new(&logo).exists() {
        println!("cargo:rerun-if-changed={logo}");
        std::fs::canonicalize(&logo)
            .expect("logoPath 无法解析")
            .to_string_lossy()
            .into_owned()
    } else {
        let out_dir = std::env::var("OUT_DIR").unwrap();
        let p = std::path::Path::new(&out_dir).join("empty_logo.bin");
        std::fs::write(&p, b"").unwrap();
        p.to_string_lossy().into_owned()
    };
    println!("cargo:rustc-env=LMM_LOGO_PATH={logo_path}");

    // 编译期配置（launcher.json）：嵌入 exe 图标 + manifest（Windows）。
    // 注意只能调用一次 compile()——winresource 每次都会生成 VERSION 资源，
    // 调两次会产生重复资源导致 LNK1123。
    #[cfg(windows)]
    {
        let config = std::fs::read_to_string("launcher.json").expect("读取 launcher.json 失败");
        let icon = serde_json::from_str::<serde_json::Value>(&config)
            .ok()
            .and_then(|v| v["icon"].as_str().map(String::from))
            .unwrap_or_default();
        let mut res = winresource::WindowsResource::new();
        if !icon.is_empty() && std::path::Path::new(&icon).exists() {
            println!("cargo:rerun-if-changed={icon}");
            res.set_icon(&icon);
        }
        // manifest：common controls 6.0（原生控件主题化）+ PerMonitorV2 DPI
        println!("cargo:rerun-if-changed=app.manifest");
        let manifest = std::fs::read_to_string("app.manifest").expect("读取 app.manifest 失败");
        res.set_manifest(&manifest);
        res.compile().expect("嵌入图标/manifest 失败");
    }
}
