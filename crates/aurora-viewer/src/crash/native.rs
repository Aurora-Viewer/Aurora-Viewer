//! Small platform boundary: wait on the actual process, not a reused PID.
//! The native exception hook only writes to a pre-opened file; it does not
//! start processes, take application locks, initialize symbols or use the GPU.

#[cfg(windows)]
mod platform {
    use std::io::Write;
    use std::path::Path;
    use std::sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, FILETIME, HANDLE, WAIT_OBJECT_0},
        System::{
            Diagnostics::Debug::{
                EXCEPTION_EXECUTE_HANDLER, EXCEPTION_POINTERS, RaiseException, RtlCaptureStackBackTrace, SetUnhandledExceptionFilter,
            },
            Threading::{
                GetCurrentProcess, GetExitCodeProcess, GetProcessTimes, INFINITE, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
                PROCESS_SYNCHRONIZE, WaitForSingleObject,
            },
        },
    };

    static TRACE: OnceLock<std::fs::File> = OnceLock::new();
    static WRITING: AtomicBool = AtomicBool::new(false);

    struct Process(HANDLE);
    impl Drop for Process {
        fn drop(&mut self) {
            // SAFETY: this wrapper exclusively owns the handle from OpenProcess.
            unsafe { CloseHandle(self.0) };
        }
    }

    fn open(pid: u32) -> Option<Process> {
        // SAFETY: no borrowed pointers; access is limited to waiting / metadata.
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        (!handle.is_null()).then_some(Process(handle))
    }

    fn started(handle: HANDLE) -> u64 {
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        // SAFETY: handle is live; all output pointers refer to local FILETIMEs.
        if unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
            return 0;
        }
        (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime)
    }

    pub fn current_process_started() -> u64 {
        // SAFETY: the pseudo-handle needs no close and is valid in this process.
        started(unsafe { GetCurrentProcess() })
    }

    pub fn is_alive(pid: u32, creation: u64) -> bool {
        let Some(process) = open(pid) else { return false };
        // SAFETY: Process owns a valid handle; this is a nonblocking query.
        started(process.0) == creation && unsafe { WaitForSingleObject(process.0, 0) } != WAIT_OBJECT_0
    }

    pub fn watch(dir: &Path, record: &super::super::Record) -> anyhow::Result<()> {
        let process = open(record.pid).ok_or_else(|| anyhow::anyhow!("cannot open viewer process"))?;
        if started(process.0) != record.process_started {
            return super::super::show_once(dir);
        }
        std::fs::write(dir.join("watching"), [])?;
        // SAFETY: the helper owns this process handle until after the wait.
        anyhow::ensure!(
            unsafe { WaitForSingleObject(process.0, INFINITE) } == WAIT_OBJECT_0,
            "cannot wait for viewer"
        );
        let mut code = 1;
        // SAFETY: valid process handle and output pointer.
        anyhow::ensure!(unsafe { GetExitCodeProcess(process.0, &mut code) } != 0, "cannot query viewer exit");
        if super::super::should_report(dir, code) {
            super::super::preserve(dir, code)?;
            super::super::show_once(dir)?;
        } else {
            // The viewer has exited and closed its files. This directory is
            // the validated UUID session supplied by our own parent only.
            let _ = std::fs::remove_dir_all(dir);
        }
        Ok(())
    }

    pub fn install(dir: &Path) -> anyhow::Result<()> {
        let file = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("native.txt"))?;
        let _ = TRACE.set(file);
        // SAFETY: callback has the ABI expected by Windows and only accesses
        // static storage that lives until process termination.
        unsafe { SetUnhandledExceptionFilter(Some(exception)) };
        Ok(())
    }

    unsafe extern "system" fn exception(info: *const EXCEPTION_POINTERS) -> i32 {
        if !WRITING.swap(true, Ordering::Relaxed) && !info.is_null() {
            // SAFETY: Windows supplies live exception/context records for the
            // duration of the callback. Check the nested pointer before use.
            let record = unsafe { (*info).ExceptionRecord };
            if let Some(mut file) = TRACE.get()
                && !record.is_null()
            {
                // SAFETY: checked above; formatting only writes to a File and
                // uses fixed stack storage, with no application/logger locks.
                let record = unsafe { &*record };
                let _ = writeln!(
                    file,
                    "Native exception 0x{:08X}\nFault address: {:p}\nNative stack (addresses):",
                    record.ExceptionCode as u32, record.ExceptionAddress
                );
                let mut addresses = [std::ptr::null_mut(); 64];
                // SAFETY: addresses has room for all 64 frames; hash is optional.
                let count = unsafe { RtlCaptureStackBackTrace(0, addresses.len() as u32, addresses.as_mut_ptr(), std::ptr::null_mut()) };
                for (index, address) in addresses[..usize::from(count)].iter().enumerate() {
                    let _ = writeln!(file, "{index:02}: {:p}", *address);
                }
                let _ = file.sync_all();
            }
        }
        EXCEPTION_EXECUTE_HANDLER
    }

    pub fn demo_exception() -> ! {
        // SAFETY: an intentional, noncontinuable SEH exception in the offline
        // test process; no invalid Rust pointer / undefined behavior involved.
        unsafe { RaiseException(0xE0424155, 1, 0, std::ptr::null()) };
        std::process::abort()
    }
}

#[cfg(not(windows))]
mod platform {
    use std::path::Path;
    pub fn current_process_started() -> u64 {
        0
    }
    pub fn is_alive(_pid: u32, _creation: u64) -> bool {
        // Automatic monitoring is currently Windows-only, like the releases.
        // Do not misidentify another live instance as crashed on other hosts.
        true
    }
    pub fn install(_dir: &Path) -> anyhow::Result<()> {
        Ok(())
    }
    pub fn watch(_dir: &Path, _record: &super::super::Record) -> anyhow::Result<()> {
        anyhow::bail!("automatic crash monitoring currently requires Windows")
    }
    pub fn demo_exception() -> ! {
        std::process::abort()
    }
}

pub use platform::*;
