//! Windows implementation: a [Job Object] with kill-on-close.
//!
//! [Job Object]: https://learn.microsoft.com/windows/win32/procthread/job-objects

use std::io;
use std::os::windows::io::AsRawHandle;
use std::process::Command;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};

use crate::Mechanism;

pub struct Job {
    handle: HANDLE,
}

// The handle is owned solely by this struct and all Win32 job APIs are
// thread-safe, so the raw pointer is fine to send/share across threads.
unsafe impl Send for Job {}
unsafe impl Sync for Job {}

impl Job {
    pub fn new() -> io::Result<Self> {
        // SAFETY: null name/attributes request an unnamed job with defaults.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Job { handle };

        // Kill every process in the job once the last handle closes — i.e. when
        // this struct drops or the parent process dies. This is the Windows
        // analogue of cgroup.kill / killpg.
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `info` is a fully-initialised struct matching the info class,
        // and its size is passed explicitly.
        let ok = unsafe {
            SetInformationJobObject(
                job.handle,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&info).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }

    pub fn spawn(&self, cmd: &mut Command) -> io::Result<std::process::Child> {
        // Spawn first, then assign. There's a small window between spawn and
        // assignment in which the child could spawn its own children outside the
        // job; acceptable for v1 (git/jj/gh don't fork that fast). Hardening via
        // CREATE_SUSPENDED + ResumeThread is a follow-up.
        let child = cmd.spawn()?;
        // SAFETY: the child handle stays valid until `child` is dropped, well
        // after this call returns.
        let ok = unsafe { AssignProcessToJobObject(self.handle, child.as_raw_handle() as HANDLE) };
        if ok == 0 {
            let err = io::Error::last_os_error();
            // Don't leak a child we failed to contain.
            let mut child = child;
            let _ = child.kill();
            return Err(err);
        }
        Ok(child)
    }

    pub fn kill_all(&self) -> io::Result<()> {
        // SAFETY: `self.handle` is a valid job handle for the lifetime of self.
        let ok = unsafe { TerminateJobObject(self.handle, 1) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn mechanism(&self) -> Mechanism {
        Mechanism::JobObject
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        // Closing the last handle triggers KILL_ON_JOB_CLOSE → the tree is reaped.
        // SAFETY: handle came from CreateJobObjectW and is closed exactly once.
        unsafe { CloseHandle(self.handle) };
    }
}
