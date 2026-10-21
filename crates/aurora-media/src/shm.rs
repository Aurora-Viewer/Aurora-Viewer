//! Shared memory segments for media pixels (LLPluginSharedMemory): the parent
//! creates a named segment, the plugin opens it by name and renders into it.
//! Windows: a CreateFileMapping named "Local\LL_<pid>_<n>". Linux: a POSIX
//! shm_open object named "/LL<pid>_<n>" (USE_SHM_OPEN_SHARED_MEMORY); the
//! plugin unlinks the name once it has opened it, so a memfd cannot be used:
//! the protocol passes a name, not a descriptor.

use std::sync::atomic::{AtomicU32, Ordering};

static SEGMENT_NUMBER: AtomicU32 = AtomicU32::new(0);

pub struct SharedMemory {
    name: String,
    size: usize,
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
    #[cfg(unix)]
    // closed after the unmapping, when the segment drops
    _fd: std::os::fd::OwnedFd,
    addr: *mut u8,
}

// the mapping is only accessed through &self / &mut self
unsafe impl Send for SharedMemory {}

impl SharedMemory {
    #[cfg(windows)]
    pub fn create(size: usize) -> Option<SharedMemory> {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Memory::{CreateFileMappingA, FILE_MAP_ALL_ACCESS, MapViewOfFile, PAGE_READWRITE};
        let name = format!(
            "Local\\LL_{}_{}",
            std::process::id(),
            SEGMENT_NUMBER.fetch_add(1, Ordering::Relaxed)
        );
        let cname = std::ffi::CString::new(name.clone()).ok()?;
        unsafe {
            let handle = CreateFileMappingA(
                INVALID_HANDLE_VALUE,
                std::ptr::null(),
                PAGE_READWRITE,
                0,
                size as u32,
                cname.as_ptr() as *const u8,
            );
            if handle.is_null() {
                log::warn!("media: CreateFileMapping failed ({})", std::io::Error::last_os_error());
                return None;
            }
            let view = MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, size);
            if view.Value.is_null() {
                log::warn!("media: MapViewOfFile failed ({})", std::io::Error::last_os_error());
                CloseHandle(handle);
                return None;
            }
            let addr = view.Value as *mut u8;
            // cleared to avoid random visual fuzz (LLPluginClassMedia::idle)
            std::ptr::write_bytes(addr, 0, size);
            Some(SharedMemory { name, size, handle, addr })
        }
    }

    #[cfg(unix)]
    pub fn create(size: usize) -> Option<SharedMemory> {
        use std::os::fd::{FromRawFd, OwnedFd};
        let name = format!("/LL{}_{}", std::process::id(), SEGMENT_NUMBER.fetch_add(1, Ordering::Relaxed));
        let cname = std::ffi::CString::new(name.clone()).ok()?;
        // SAFETY: cname is a valid C string; the descriptor is owned by the
        // OwnedFd right away; the mapping covers `size` bytes of an object
        // truncated to that size, and is only released in Drop.
        unsafe {
            // preemptive unlink, as LLPluginSharedMemory::create
            libc::shm_unlink(cname.as_ptr());
            let raw = libc::shm_open(cname.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o600);
            if raw == -1 {
                log::warn!("media: shm_open failed ({})", std::io::Error::last_os_error());
                return None;
            }
            let fd = OwnedFd::from_raw_fd(raw);
            fn fail(what: &str, name: &std::ffi::CStr) -> Option<SharedMemory> {
                log::warn!("media: {what} failed ({})", std::io::Error::last_os_error());
                // SAFETY: a valid C string
                unsafe { libc::shm_unlink(name.as_ptr()) };
                None
            }
            let Ok(len) = libc::off_t::try_from(size) else {
                return fail("shared memory size", &cname);
            };
            if libc::ftruncate(raw, len) == -1 {
                return fail("ftruncate", &cname);
            }
            let addr = libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                raw,
                0,
            );
            if addr == libc::MAP_FAILED {
                return fail("mmap", &cname);
            }
            // a new object reads as zeros already: no clearing needed
            Some(SharedMemory {
                name,
                size,
                _fd: fd,
                addr: addr as *mut u8,
            })
        }
    }

    #[cfg(not(any(windows, unix)))]
    pub fn create(_size: usize) -> Option<SharedMemory> {
        let _ = SEGMENT_NUMBER.load(Ordering::Relaxed);
        None
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn bytes(&self) -> &[u8] {
        // SAFETY: mapped for `size` bytes for the lifetime of self; the plugin
        // writes concurrently (torn frames are accepted, as in LL)
        unsafe { std::slice::from_raw_parts(self.addr, self.size) }
    }
}

impl Drop for SharedMemory {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::Foundation::CloseHandle;
            use windows_sys::Win32::System::Memory::{MEMORY_MAPPED_VIEW_ADDRESS, UnmapViewOfFile};
            UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                Value: self.addr as *mut _,
            });
            CloseHandle(self.handle);
        }
        // SAFETY: addr / size are the mapping made in `create`; the name is
        // usually unlinked by the plugin already (an error is then expected).
        // The descriptor closes with `_fd`.
        #[cfg(unix)]
        unsafe {
            libc::munmap(self.addr as *mut libc::c_void, self.size);
            if let Ok(cname) = std::ffi::CString::new(self.name.clone()) {
                libc::shm_unlink(cname.as_ptr());
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_segment_is_shared_by_name_like_the_plugin_opens_it() {
        let seg = SharedMemory::create(4096).expect("segment");
        assert!(seg.name().starts_with(&format!("/LL{}_", std::process::id())));
        // what LLPluginSharedMemory::attach does in the plugin
        let cname = std::ffi::CString::new(seg.name()).expect("name");
        // SAFETY: test-only mapping of the segment just created, 4096 bytes
        unsafe {
            let fd = libc::shm_open(cname.as_ptr(), libc::O_RDWR, 0o600);
            assert!(fd >= 0);
            let other = libc::mmap(
                std::ptr::null_mut(),
                4096,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            );
            assert_ne!(other, libc::MAP_FAILED);
            *(other as *mut u8).add(100) = 42;
            libc::munmap(other, 4096);
            libc::close(fd);
        }
        assert_eq!(seg.bytes()[100], 42);
        assert!(seg.bytes()[..100].iter().all(|&b| b == 0));
    }
}
