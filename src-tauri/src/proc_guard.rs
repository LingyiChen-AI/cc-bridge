//! 让代理子进程的生命周期绑定到本客户端：客户端无论怎么退出，代理都不会变成孤儿。
use std::process::{Child, Command};

/// spawn 之前调用（Linux 设置 PR_SET_PDEATHSIG；Windows 隐藏控制台窗口）。
pub fn prepare(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(target_os = "linux")]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    let _ = cmd;
}

/// spawn 之后调用。Windows：放进 KILL_ON_JOB_CLOSE 的 Job；其它平台靠退出时 stop()。
pub fn attach_kill_on_close(child: &Child) {
    #[cfg(windows)]
    windows_job::attach(child);
    #[cfg(not(windows))]
    let _ = child;
}

#[cfg(windows)]
mod windows_job {
    use std::{os::windows::io::AsRawHandle, process::Child, sync::OnceLock};
    use windows_sys::Win32::System::JobObjects::*;

    struct Job(isize);
    // 句柄仅用于 AssignProcessToJobObject，进程退出时由系统关闭 → 触发 KILL_ON_JOB_CLOSE
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}
    static JOB: OnceLock<Job> = OnceLock::new();

    fn job() -> isize {
        JOB.get_or_init(|| unsafe {
            let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            Job(h as isize)
        })
        .0
    }

    pub fn attach(child: &Child) {
        unsafe {
            AssignProcessToJobObject(job() as _, child.as_raw_handle() as _);
        }
    }
}
