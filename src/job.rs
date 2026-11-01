//! Windows Job Object：把启动器 spawn 的所有子进程（pip / 主程序）挂进同一个 job，
//! 并设置 KILL_ON_JOB_CLOSE——启动器退出（无论正常关闭还是被强杀）时，
//! job 句柄随进程销毁关闭，系统自动终止全部子进程，避免孤儿进程残留。

#![cfg(windows)]

use std::os::windows::io::AsRawHandle;
use std::process::Child;
use std::sync::OnceLock;

#[link(name = "kernel32")]
extern "system" {
    fn CreateJobObjectW(lp_attributes: *const core::ffi::c_void, lp_name: *const u16) -> *mut core::ffi::c_void;
    fn SetInformationJobObject(
        h_job: *mut core::ffi::c_void,
        info_class: i32,
        info: *const core::ffi::c_void,
        len: u32,
    ) -> i32;
    fn AssignProcessToJobObject(h_job: *mut core::ffi::c_void, h_proc: *mut core::ffi::c_void) -> i32;
}

const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: i32 = 9;
const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;

#[repr(C)]
#[derive(Default)]
struct JOBOBJECT_BASIC_LIMIT_INFORMATION {
    per_process_user_time_limit: i64,
    per_job_user_time_limit: i64,
    limit_flags: u32,
    minimum_working_set_size: usize,
    maximum_working_set_size: usize,
    active_process_limit: u32,
    affinity: usize,
    priority_class: u32,
    scheduling_class: u32,
}

#[repr(C)]
struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
    basic: JOBOBJECT_BASIC_LIMIT_INFORMATION,
    io_info: [u64; 8],
    process_memory_limit: usize,
    job_memory_limit: usize,
    peak_process_memory_used: usize,
    peak_job_memory_used: usize,
}

static JOB: OnceLock<usize> = OnceLock::new();

/// 进程启动时调用一次：创建 job 并设置 KILL_ON_JOB_CLOSE。
/// 失败仅记日志，不阻断启动（退化为无 job 保护）。
pub fn init() {
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job.is_null() {
        crate::logging::error(crate::i18n::strings().job_create_fail);
        return;
    }
    let info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
        basic: JOBOBJECT_BASIC_LIMIT_INFORMATION {
            limit_flags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            ..Default::default()
        },
        io_info: [0; 8],
        process_memory_limit: 0,
        job_memory_limit: 0,
        peak_process_memory_used: 0,
        peak_job_memory_used: 0,
    };
    let ok = unsafe {
        SetInformationJobObject(
            job,
            JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
            &info as *const _ as *const _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if ok == 0 {
        crate::logging::error(crate::i18n::strings().job_limit_fail);
        return;
    }
    let _ = JOB.set(job as usize);
}

/// 把已 spawn 的子进程挂进 job。挂入失败不影响子进程继续运行。
pub fn assign_child(child: &Child) {
    let Some(&job) = JOB.get() else { return };
    let ok = unsafe { AssignProcessToJobObject(job as *mut _, child.as_raw_handle() as *mut _) };
    if ok == 0 {
        crate::logging::error(crate::i18n::strings().job_assign_fail);
    }
}
