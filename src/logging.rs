//! 简易文件日志：写入 logs/launcher-<时间戳>.log
//!
//! 时间通过 Win32 GetLocalTime 获取，避免引入 chrono 依赖。

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::{Mutex, OnceLock};

static LOGGER: OnceLock<Mutex<File>> = OnceLock::new();

#[cfg(windows)]
mod local_time {
    #[repr(C)]
    pub struct SystemTime {
        pub year: u16,
        pub month: u16,
        pub day_of_week: u16,
        pub day: u16,
        pub hour: u16,
        pub minute: u16,
        pub second: u16,
        pub milliseconds: u16,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetLocalTime(lpSystemTime: *mut SystemTime);
    }
    pub fn now() -> SystemTime {
        let mut t = SystemTime {
            year: 0, month: 0, day_of_week: 0, day: 0,
            hour: 0, minute: 0, second: 0, milliseconds: 0,
        };
        unsafe { GetLocalTime(&mut t) };
        t
    }
}

/// 当前本地时间，格式 (YYYY, MM, DD, HH, MM, SS)
fn now_parts() -> (i32, u32, u32, u32, u32, u32) {
    #[cfg(windows)]
    {
        let t = local_time::now();
        (t.year as i32, t.month as u32, t.day as u32, t.hour as u32, t.minute as u32, t.second as u32)
    }
    #[cfg(not(windows))]
    {
        // 非 Windows：退化用 UNIX 时间戳换算（启动器只在 Windows 分发）
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let days = secs / 86400;
        let rem = secs % 86400;
        let (year, month, day) = civil_from_days(days as i64);
        (year, month, day, (rem / 3600) as u32, ((rem % 3600) / 60) as u32, (rem % 60) as u32)
    }
}

#[cfg(not(windows))]
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

pub fn init() -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all("logs")?;
    let (y, mo, d, h, mi, s) = now_parts();
    let path = format!("logs/launcher-{y:04}-{mo:02}-{d:02}-{h:02}-{mi:02}-{s:02}.log");
    let file = OpenOptions::new().create(true).write(true).truncate(true).open(path)?;
    let _ = LOGGER.set(Mutex::new(file));
    info(&format!("启动器版本: {}", env!("CARGO_PKG_VERSION")));
    Ok(())
}

fn write_line(level: &str, msg: &str) {
    if let Some(m) = LOGGER.get() {
        if let Ok(mut f) = m.lock() {
            let (y, mo, d, h, mi, s) = now_parts();
            let _ = writeln!(f, "[{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}][{level}] {msg}");
        }
    }
}

pub fn info(msg: &str) {
    write_line("INFO", msg);
}

pub fn error(msg: &str) {
    write_line("ERROR", msg);
}
