//! Shared memory segments for media pixels (LLPluginSharedMemory, Win32
//! CreateFileMapping implementation): the parent creates a named mapping
//! "Local\LL_<pid>_<n>", the plugin opens it by name and renders into it.

use std::sync::atomic::{AtomicU32, Ordering};

static SEGMENT_NUMBER: AtomicU32 = AtomicU32::new(0);

pub struct SharedMemory {
    name: String,
    size: usize,
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
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

    #[cfg(not(windows))]
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
    }
}
