//! Small platform boundary: wait on the actual process, not a reused PID.
//! The native exception hook only writes to a pre-opened file; it does not
//! start processes, take application locks, initialize symbols or use the GPU.

#[cfg(windows)]
mod platform {
    use std::ffi::c_void;
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
                EXCEPTION_EXECUTE_HANDLER, EXCEPTION_POINTERS, IMAGE_DEBUG_DIRECTORY, IMAGE_DEBUG_TYPE_CODEVIEW,
                IMAGE_DIRECTORY_ENTRY_DEBUG, IMAGE_NT_HEADERS64, IMAGE_NT_OPTIONAL_HDR64_MAGIC, RaiseException, RtlCaptureStackBackTrace,
                RtlPcToFileHeader, SetUnhandledExceptionFilter,
            },
            Threading::{
                GetCurrentProcess, GetExitCodeProcess, GetProcessTimes, INFINITE, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
                PROCESS_SYNCHRONIZE, WaitForSingleObject,
            },
        },
    };

    static TRACE: OnceLock<std::fs::File> = OnceLock::new();
    static WRITING: AtomicBool = AtomicBool::new(false);
    const FRAMES: usize = 64;

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
                let mut addresses = [std::ptr::null_mut(); FRAMES];
                // SAFETY: addresses has room for all 64 frames; hash is optional.
                let count = unsafe { RtlCaptureStackBackTrace(0, addresses.len() as u32, addresses.as_mut_ptr(), std::ptr::null_mut()) };
                let frames = &addresses[..usize::from(count)];
                for (index, address) in frames.iter().enumerate() {
                    let _ = writeln!(file, "{index:02}: {:p}", *address);
                }
                let _ = file.sync_all();
                // The module headers are read once the raw stack is on disk:
                // a fault while reading them only loses the module names.
                write_modules(&mut file, frames, Some(record.ExceptionAddress));
                let _ = file.sync_all();
            }
        }
        EXCEPTION_EXECUTE_HANDLER
    }

    /// The current thread's stack as module + offset, for a Rust panic.
    pub fn write_stack(out: &mut impl Write) {
        let mut addresses = [std::ptr::null_mut(); FRAMES];
        // SAFETY: addresses has room for all 64 frames; hash is optional.
        let count = unsafe { RtlCaptureStackBackTrace(1, addresses.len() as u32, addresses.as_mut_ptr(), std::ptr::null_mut()) };
        write_modules(out, &addresses[..usize::from(count)], None);
    }

    /// Raw addresses change with ASLR at every launch: only module + offset,
    /// with each module's PDB identity, can be symbolized once the process
    /// is gone. No allocation, no loader lock, no symbol engine: this runs
    /// inside the exception filter.
    pub(super) fn write_modules(out: &mut impl Write, frames: &[*mut c_void], fault: Option<*mut c_void>) {
        let mut bases = [0usize; FRAMES + 1];
        let mut known = 0;
        let mut seen = |base: usize| {
            if base != 0 && !bases[..known].contains(&base) && known < bases.len() {
                bases[known] = base;
                known += 1;
            }
        };
        if let Some(fault) = fault {
            let _ = write!(out, "Fault: ");
            seen(write_location(out, fault));
        }
        let _ = writeln!(out, "Native stack (modules):");
        for (index, &address) in frames.iter().enumerate() {
            let _ = write!(out, "{index:02}: ");
            seen(write_location(out, address));
        }
        let _ = writeln!(out, "Modules:");
        for &base in &bases[..known] {
            // SAFETY: base came from RtlPcToFileHeader for a frame of this
            // process; a module on the stack is not unloaded under it.
            let Some(module) = (unsafe { Module::read(base) }) else { continue };
            let _ = write!(out, "{}", module.name());
            if let Some(guid) = module.guid {
                let data1 = u32::from_le_bytes([guid[0], guid[1], guid[2], guid[3]]);
                let data2 = u16::from_le_bytes([guid[4], guid[5]]);
                let data3 = u16::from_le_bytes([guid[6], guid[7]]);
                let _ = write!(out, " id={data1:08X}{data2:04X}{data3:04X}");
                for byte in &guid[8..] {
                    let _ = write!(out, "{byte:02X}");
                }
                let _ = write!(out, "{:X}", module.age);
            }
            // Code id, as symbol servers key the binary itself.
            let _ = writeln!(out, " code={:08X}{:x}", module.stamp, module.size);
        }
    }

    /// Writes `name+0xoffset` (or `?` outside any module) and a line end;
    /// returns the module base, 0 when unknown.
    fn write_location(out: &mut impl Write, address: *mut c_void) -> usize {
        let mut base = std::ptr::null_mut();
        // SAFETY: a lookup in the loader's inverted function table, the one
        // exception dispatch itself has just used to reach this filter.
        unsafe { RtlPcToFileHeader(address, &mut base) };
        let base = base as usize;
        // SAFETY: a non-null base is the start of an image mapped here.
        match (base != 0).then(|| unsafe { Module::read(base) }).flatten() {
            Some(module) => {
                let _ = writeln!(out, "{}+0x{:x}", module.name(), address as usize - base);
                base
            }
            None => {
                let _ = writeln!(out, "?");
                0
            }
        }
    }

    struct Module {
        size: u32,
        stamp: u32,
        // CodeView (RSDS) signature and age: the key of the module's PDB and
        // of the symbol files made from it.
        guid: Option<[u8; 16]>,
        age: u32,
        name: [u8; 64],
        len: usize,
    }

    impl Module {
        fn name(&self) -> &str {
            match std::str::from_utf8(&self.name[..self.len]) {
                Ok(name) if !name.is_empty() => name,
                _ => "module",
            }
        }

        /// Reads the PE headers and the CodeView record where the loader
        /// mapped them, without opening the file.
        ///
        /// # Safety
        /// `base` is the start of an image mapped in this process: its first
        /// page and every range inside SizeOfImage that the headers name are
        /// readable.
        unsafe fn read(base: usize) -> Option<Module> {
            const PAGE: usize = 4096;
            let at = |offset: usize| (base + offset) as *const u8;
            // SAFETY: the headers' first page is readable (see above).
            let nt = unsafe {
                if std::ptr::read_unaligned(at(0).cast::<u16>()) != 0x5A4D {
                    return None;
                }
                std::ptr::read_unaligned(at(0x3C).cast::<u32>()) as usize
            };
            if nt + size_of::<IMAGE_NT_HEADERS64>() > PAGE {
                return None;
            }
            // SAFETY: bounded to the first page just above.
            let headers = unsafe { std::ptr::read_unaligned(at(nt).cast::<IMAGE_NT_HEADERS64>()) };
            let optional = headers.OptionalHeader;
            if headers.Signature != 0x4550 || optional.Magic != IMAGE_NT_OPTIONAL_HDR64_MAGIC {
                return None;
            }
            let size = optional.SizeOfImage as usize;
            let mut module = Module {
                size: optional.SizeOfImage,
                stamp: headers.FileHeader.TimeDateStamp,
                guid: None,
                age: 0,
                name: [0; 64],
                len: 0,
            };
            let index = IMAGE_DIRECTORY_ENTRY_DEBUG as usize;
            if optional.NumberOfRvaAndSizes as usize <= index {
                return Some(module);
            }
            let directory = optional.DataDirectory[index];
            let (start, length) = (directory.VirtualAddress as usize, directory.Size as usize);
            if start == 0 || start + length > size {
                return Some(module);
            }
            let entry = size_of::<IMAGE_DEBUG_DIRECTORY>();
            for i in 0..(length / entry).min(16) {
                // SAFETY: inside the debug directory, itself inside the image.
                let debug = unsafe { std::ptr::read_unaligned(at(start + i * entry).cast::<IMAGE_DEBUG_DIRECTORY>()) };
                let (data, length) = (debug.AddressOfRawData as usize, debug.SizeOfData as usize);
                if debug.Type != IMAGE_DEBUG_TYPE_CODEVIEW || data == 0 || length < 24 || data + length > size {
                    continue;
                }
                // SAFETY: the CodeView record lies inside the image (checked).
                let record = unsafe { std::slice::from_raw_parts(at(data), length) };
                if &record[..4] != b"RSDS" {
                    continue;
                }
                module.guid = record[4..20].try_into().ok();
                module.age = u32::from_le_bytes([record[20], record[21], record[22], record[23]]);
                let path = &record[24..];
                let path = &path[..path.iter().position(|&b| b == 0).unwrap_or(path.len())];
                // Only the file name: a local build's PDB path names the
                // developer's folders.
                let file = &path[path.iter().rposition(|&b| b == b'\\' || b == b'/').map_or(0, |i| i + 1)..];
                let stem = match file.len().checked_sub(4) {
                    Some(dot) if file[dot..].eq_ignore_ascii_case(b".pdb") => &file[..dot],
                    _ => file,
                };
                for &byte in stem.iter().take(module.name.len()) {
                    let allowed = byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.');
                    module.name[module.len] = if allowed { byte } else { b'_' };
                    module.len += 1;
                }
                break;
            }
            Some(module)
        }
    }

    pub fn demo_exception() -> ! {
        // SAFETY: an intentional, noncontinuable SEH exception in the offline
        // test process; no invalid Rust pointer / undefined behavior involved.
        unsafe { RaiseException(0xE0424155, 1, 0, std::ptr::null()) };
        std::process::abort()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn modules(text: &str) -> Vec<&str> {
            text.split_once("Modules:\n")
                .map(|(_, list)| list.lines().collect())
                .unwrap_or_default()
        }

        #[test]
        fn stacks_are_written_as_module_offsets_with_symbol_identities() {
            let mut out = Vec::new();
            write_stack(&mut out);
            let text = String::from_utf8(out).unwrap();
            let frames: Vec<_> = text.lines().filter(|line| line.contains("+0x")).collect();
            assert!(frames.len() > 3, "{text}");
            // Offsets inside their module, never absolute addresses: those
            // differ at every launch.
            for frame in &frames {
                let offset = frame.rsplit_once("+0x").unwrap().1;
                assert!(u64::from_str_radix(offset, 16).unwrap() < 1 << 32, "{frame}");
            }
            let modules = modules(&text);
            assert!(modules.iter().any(|line| line.starts_with("ntdll ")), "{text}");
            // This test binary has a PDB: GUID (32 hex digits) and age.
            let own = modules.iter().find(|line| line.starts_with("aurora_viewer")).expect("own module");
            let id = own.split(' ').find_map(|part| part.strip_prefix("id=")).expect("debug id");
            assert!(id.len() >= 33 && id.chars().all(|c| c.is_ascii_hexdigit()), "{own}");
            assert!(own.contains(" code="), "{own}");
        }

        #[test]
        fn the_fault_location_and_unknown_frames_are_written() {
            let here = stacks_are_written_as_module_offsets_with_symbol_identities as *mut c_void;
            let mut out = Vec::new();
            write_modules(&mut out, &[here, std::ptr::null_mut()], Some(here));
            let text = String::from_utf8(out).unwrap();
            let mut lines = text.lines();
            let fault = lines.next().unwrap();
            assert!(fault.starts_with("Fault: aurora_viewer") && fault.contains("+0x"), "{text}");
            assert_eq!(lines.next(), Some("Native stack (modules):"));
            assert_eq!(lines.next(), Some(&*format!("00: {}", &fault["Fault: ".len()..])));
            assert_eq!(lines.next(), Some("01: ?"));
            assert_eq!(modules(&text).len(), 1, "{text}");
        }
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
    pub fn write_stack(_out: &mut impl std::io::Write) {}
    pub fn watch(_dir: &Path, _record: &super::super::Record) -> anyhow::Result<()> {
        anyhow::bail!("automatic crash monitoring currently requires Windows")
    }
    pub fn demo_exception() -> ! {
        std::process::abort()
    }
}

pub use platform::*;
