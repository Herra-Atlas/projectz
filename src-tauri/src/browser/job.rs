//! Making the browser die with the app, on Windows.
//!
//! # Why a job object
//!
//! The browser is started as a child of this process, and [`super::terminate`]
//! kills its whole tree at exit -- which covers a quit the app performs itself.
//! It does not cover the ways a process *stops* rather than quits: a crash, a
//! force-quit from Task Manager, a `Ctrl+C` in `tauri dev`, a session ending. In
//! every one of those the exit handler never runs and a Chrome window is left
//! behind, indistinguishable from one the user opened themselves -- which is the
//! failure this module exists to close.
//!
//! A job object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` is the operating
//! system's own answer: the browser is placed in a job this process owns, the job
//! handle is held for the life of the process, and the kernel kills everything in
//! the job the instant the last handle to it closes -- which is when this process
//! dies, however it dies. `ai/tools/process.rs` names the same mechanism as the
//! fix for a reparented background command; here the process is a browser that
//! must never outlive the app that opened it.
//!
//! Everything here is best-effort. A machine that refuses the job -- an older
//! Windows, a sandbox that forbids nesting -- still has [`super::terminate`] for
//! the graceful path, so a failure is logged and the browser carries on rather
//! than the run failing over a guarantee it did not need yet.

/// Puts `pid` in a job that is killed when this process ends.
///
/// Called once per browser, so a browser replaced after a crash is adopted the
/// same way the first one was.
#[cfg(windows)]
pub(super) fn adopt(pid: u32) {
    use core::ffi::c_void;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };

    /// The job, created once and held for the life of the process.
    ///
    /// Held rather than closed after assigning: the limit fires when the *last*
    /// handle closes, so dropping it here would kill the browser immediately. It is
    /// deliberately leaked -- the OS releases it at exit, which is exactly the
    /// moment it should fire. Zero means the job could not be made.
    static JOB: std::sync::OnceLock<isize> = std::sync::OnceLock::new();

    let job = *JOB.get_or_init(|| unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            tracing::warn!("the browser could not be given a job object");
            return 0;
        }
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let armed = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if armed == 0 {
            tracing::warn!("the browser's job object could not be armed");
            let _ = CloseHandle(job);
            return 0;
        }
        job as isize
    });
    if job == 0 {
        return;
    }

    unsafe {
        // `PROCESS_SET_QUOTA` is what assigning to a job requires; `PROCESS_TERMINATE`
        // is the access the job's own kill needs to take effect.
        let process: HANDLE = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
        if process.is_null() {
            tracing::warn!(pid, "the browser's process could not be opened to adopt it");
            return;
        }
        let assigned = AssignProcessToJobObject(job as HANDLE, process);
        let _ = CloseHandle(process);
        if assigned == 0 {
            tracing::warn!(pid, "the browser could not be assigned to its job object");
        }
    }
}

/// Nothing to do off Windows, where [`super::terminate`] tears the tree down.
#[cfg(not(windows))]
pub(super) fn adopt(_pid: u32) {}
