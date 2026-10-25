//! The process's memory allocator: mimalloc, or the system's with
//! `AURORA_ALLOCATOR=system`.
//!
//! The system allocator (on Windows the process heap) serves every thread
//! from one heap behind one lock. When the background jobs decode and build
//! in a burst (the camera turning in a dense place: hundreds of jobs, up to
//! 13 cores busy), the main thread queued behind them at almost every
//! allocation and free of its frame. In an ETW trace of such a stall at KB
//! (frames of 26 to 110 ms), the main thread spent 530 ms of 700 off the
//! processor, in thousands of waits of about 0.3 ms on the heap's lock:
//! growing a `Vec` alone was 196 ms of it, the rest the frees of the draw
//! lists and of the sync, egui's sorts, the lights and the poses. The render
//! thread's `frame writes` slowed down at the same moments. mimalloc gives
//! each thread its own heap: a thread frees into another's without taking a
//! lock, and a burst of jobs no longer holds the frame. Version 2 of
//! mimalloc (Cargo.toml): with version 3 the render thread lost 20 to 43 ms
//! now and then during the same bursts.
//!
//! The choice is read once, at the first allocation (before `main`, on the
//! only thread there is), without allocating; it never changes afterwards,
//! so every block goes back to the allocator that gave it.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU8, Ordering};

use mimalloc::MiMalloc;

const UNSET: u8 = 0;
const MIMALLOC: u8 = 1;
const SYSTEM: u8 = 2;

static CHOICE: AtomicU8 = AtomicU8::new(UNSET);

/// mimalloc, or the system's allocator when asked for.
pub struct Allocator;

#[global_allocator]
static GLOBAL: Allocator = Allocator;

/// The allocator in use, for the log.
pub fn name() -> &'static str {
    if use_system() { "system" } else { "mimalloc" }
}

#[inline]
fn use_system() -> bool {
    match CHOICE.load(Ordering::Relaxed) {
        MIMALLOC => false,
        SYSTEM => true,
        _ => {
            // two threads racing here read the same environment
            let system = system_wanted();
            CHOICE.store(if system { SYSTEM } else { MIMALLOC }, Ordering::Relaxed);
            system
        }
    }
}

/// `AURORA_ALLOCATOR=system`, read without allocating.
#[cfg(windows)]
fn system_wanted() -> bool {
    use windows_sys::Win32::System::Environment::GetEnvironmentVariableW;
    const fn wide<const N: usize>(s: &str) -> [u16; N] {
        let (bytes, mut out, mut i) = (s.as_bytes(), [0u16; N], 0);
        while i < bytes.len() {
            out[i] = bytes[i] as u16;
            i += 1;
        }
        out
    }
    const NAME: [u16; 17] = wide("AURORA_ALLOCATOR");
    const SYSTEM_VALUE: [u16; 6] = wide("system");
    let mut value = [0u16; 8];
    // SAFETY: NAME ends with a NUL, and the buffer's length in characters
    // is the one passed; the call writes at most that many.
    let len = unsafe { GetEnvironmentVariableW(NAME.as_ptr(), value.as_mut_ptr(), value.len() as u32) };
    len as usize == SYSTEM_VALUE.len() && value[..SYSTEM_VALUE.len()] == SYSTEM_VALUE
}

/// `AURORA_ALLOCATOR=system`, read without allocating.
#[cfg(unix)]
fn system_wanted() -> bool {
    // SAFETY: the name is a NUL-terminated literal; getenv returns null or
    // a NUL-terminated string that stays valid while nobody sets the
    // environment, which no thread does before the first allocation.
    let value = unsafe { libc::getenv(c"AURORA_ALLOCATOR".as_ptr()) };
    // SAFETY: not null, and NUL-terminated (above)
    !value.is_null() && unsafe { std::ffi::CStr::from_ptr(value) } == c"system"
}

#[cfg(not(any(windows, unix)))]
fn system_wanted() -> bool {
    false
}

// SAFETY: every call goes to one of two allocators that implement
// `GlobalAlloc` correctly, always the same one for the whole process
// (`CHOICE` is set once, before any block exists), so a block is freed or
// resized by the allocator that gave it.
unsafe impl GlobalAlloc for Allocator {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's contract, passed on unchanged
        let block = unsafe {
            if use_system() {
                System.alloc(layout)
            } else {
                MiMalloc.alloc(layout)
            }
        };
        failed(block, layout.size())
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's contract, passed on unchanged
        let block = unsafe {
            if use_system() {
                System.alloc_zeroed(layout)
            } else {
                MiMalloc.alloc_zeroed(layout)
            }
        };
        failed(block, layout.size())
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: the caller's contract, passed on unchanged; `ptr` comes
        // from the same allocator (see the impl)
        unsafe {
            if use_system() {
                System.dealloc(ptr, layout)
            } else {
                MiMalloc.dealloc(ptr, layout)
            }
        }
    }

    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller's contract, passed on unchanged; `ptr` comes
        // from the same allocator (see the impl)
        let block = unsafe {
            if use_system() {
                System.realloc(ptr, layout, new_size)
            } else {
                MiMalloc.realloc(ptr, layout, new_size)
            }
        };
        failed(block, new_size)
    }
}

/// A failed allocation is noted for the crash report (crash/native.rs,
/// without allocating) before the caller aborts: out of memory used to end
/// the viewer with no stack at all.
#[inline]
fn failed(block: *mut u8, size: usize) -> *mut u8 {
    if block.is_null() {
        crate::crash::out_of_memory(size);
    }
    block
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_choice_is_made_once_and_kept() {
        // the test binary allocated long before this line
        let first = name();
        let v: Vec<u64> = (0..10_000).collect();
        assert_eq!(v.iter().sum::<u64>(), 49_995_000);
        assert_eq!(name(), first);
        assert_ne!(CHOICE.load(Ordering::Relaxed), UNSET);
    }
}
