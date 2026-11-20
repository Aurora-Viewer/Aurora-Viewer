//! The process's open files at a GPU loss, by kind, for the crash report
//! (#2970). On Linux, Mesa reports a descriptor it could not get (the sync
//! file or syncobj of a swapchain image) as "out of memory", and wgpu then
//! loses the device: a viewer at its open-file limit looks exactly like a
//! card out of video memory. The count at the loss tells them apart, and
//! the kinds say what held the descriptors (sockets, GPU sync objects,
//! DMA buffers, files).
//!
//! At the limit no new descriptor can be had, including the one that lists
//! `/proc/self/fd`: [`prime`] opens that directory once when crash recovery
//! starts and the census rewinds it. Classifying an entry reads its link (`readlink`) and
//! the limit comes from `getrlimit`, neither of which takes a descriptor.
//! Linux only: elsewhere [`census`] is `None`.

/// The kinds counted, in the order of the line; the crash report checks
/// the labels against this list (summary.rs).
pub const OPEN_FILE_KINDS: [&str; 9] = [
    "sockets",
    "sync files",
    "syncobjs",
    "dma-bufs",
    "GPU devices",
    "pipes",
    "memfds",
    "other kernel objects",
    "files",
];

/// The kind of a descriptor from its `/proc/self/fd/<n>` link, an index
/// into [`OPEN_FILE_KINDS`].
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn kind_of(link: &str) -> usize {
    if link.starts_with("socket:") {
        0
    } else if link == "anon_inode:sync_file" {
        1
    } else if link == "anon_inode:syncobj_file" {
        2
    } else if link.starts_with("/dmabuf:") || link == "anon_inode:dmabuf" {
        3
    } else if link.starts_with("/dev/dri/") || link.starts_with("/dev/nvidia") || link == "/dev/kfd" {
        4
    } else if link.starts_with("pipe:") {
        5
    } else if link.starts_with("/memfd:") {
        6
    } else if link.starts_with("anon_inode:") {
        7
    } else {
        8
    }
}

/// The line logged at a loss: the count, the limit and the kinds held
/// (those at zero left out).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn line(counts: &[usize; OPEN_FILE_KINDS.len()], limit: Option<u64>) -> String {
    let total: usize = counts.iter().sum();
    let limit = limit.map_or_else(|| "unknown".to_owned(), |l| l.to_string());
    let kinds: Vec<String> = OPEN_FILE_KINDS
        .iter()
        .zip(counts)
        .filter(|(_, n)| **n > 0)
        .map(|(kind, n)| format!("{kind} {n}"))
        .collect();
    format!("{total} of {limit} ({})", kinds.join(", "))
}

#[cfg(target_os = "linux")]
mod imp {
    use super::{OPEN_FILE_KINDS, kind_of, line};
    use std::sync::Mutex;

    /// The `/proc/self/fd` directory, held open from startup.
    struct Held {
        dir: *mut libc::DIR,
    }
    // SAFETY: the DIR is only used under the mutex below, by one thread at a time.
    unsafe impl Send for Held {}

    /// The soft limit now (`None` when unlimited): a limit lowered while
    /// the viewer runs is the one that counts at the loss.
    fn soft_limit() -> Option<u64> {
        let mut limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        // SAFETY: a valid out pointer; the result is checked.
        if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 || limit.rlim_cur == libc::RLIM_INFINITY {
            return None;
        }
        Some(limit.rlim_cur)
    }

    static HELD: Mutex<Option<Held>> = Mutex::new(None);

    pub fn prime() {
        let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
        if held.is_some() {
            return;
        }
        // SAFETY: a NUL-terminated path; a null result is checked.
        let dir = unsafe { libc::opendir(c"/proc/self/fd".as_ptr()) };
        if dir.is_null() {
            log::warn!("open files: /proc/self/fd not opened ({})", std::io::Error::last_os_error());
            return;
        }
        *held = Some(Held { dir });
    }

    pub fn census() -> Option<String> {
        let held = HELD.lock().unwrap_or_else(|e| e.into_inner());
        let held = held.as_ref()?;
        let mut counts = [0usize; OPEN_FILE_KINDS.len()];
        // SAFETY: `dir` is the open DIR from `prime`, never closed, used
        // under the mutex; each entry is read before the next readdir.
        unsafe {
            libc::rewinddir(held.dir);
            loop {
                let entry = libc::readdir(held.dir);
                if entry.is_null() {
                    break;
                }
                let name = std::ffi::CStr::from_ptr((*entry).d_name.as_ptr());
                let Ok(name) = name.to_str() else { continue };
                if name.starts_with('.') {
                    continue;
                }
                // (readlink takes no descriptor: it works at the limit)
                let link = std::fs::read_link(format!("/proc/self/fd/{name}"))
                    .map(|l| l.to_string_lossy().into_owned())
                    .unwrap_or_default();
                counts[kind_of(&link)] += 1;
            }
        }
        Some(line(&counts, soft_limit()))
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    pub fn prime() {}

    pub fn census() -> Option<String> {
        None
    }
}

/// Opens what the census needs while descriptors are still free: called
/// once when crash recovery starts (mod.rs).
pub fn prime() {
    imp::prime();
}

/// The open files by kind, now (Linux; `None` elsewhere or when [`prime`]
/// could not open the directory).
pub fn census() -> Option<String> {
    imp::census()
}

/// Logs the census once per loss, when the viewer notices it (mod.rs):
/// the line the crash report reads ([`LINE_PREFIX`]).
pub fn log_at_loss() {
    if let Some(open) = census() {
        log::error!("{LINE_PREFIX}{open}");
    }
}

/// The start of the logged line, after the log header.
pub const LINE_PREFIX: &str = "open files at the loss: ";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptors_are_sorted_by_what_holds_them() {
        let kind = |link: &str| OPEN_FILE_KINDS[kind_of(link)];
        assert_eq!(kind("socket:[123456]"), "sockets");
        assert_eq!(kind("anon_inode:sync_file"), "sync files");
        assert_eq!(kind("anon_inode:syncobj_file"), "syncobjs");
        assert_eq!(kind("/dmabuf:"), "dma-bufs");
        assert_eq!(kind("anon_inode:dmabuf"), "dma-bufs");
        assert_eq!(kind("/dev/dri/renderD128"), "GPU devices");
        assert_eq!(kind("/dev/nvidia0"), "GPU devices");
        assert_eq!(kind("pipe:[42]"), "pipes");
        assert_eq!(kind("/memfd:wayland-shm (deleted)"), "memfds");
        assert_eq!(kind("anon_inode:[eventfd]"), "other kernel objects");
        assert_eq!(kind("/home/someone/.cache/x"), "files");
        assert_eq!(kind(""), "files");
    }

    #[test]
    fn the_line_names_the_count_the_limit_and_the_kinds_held() {
        let mut counts = [0usize; OPEN_FILE_KINDS.len()];
        counts[0] = 812;
        counts[1] = 3;
        counts[8] = 209;
        assert_eq!(line(&counts, Some(1024)), "1024 of 1024 (sockets 812, sync files 3, files 209)");
        assert_eq!(line(&[0; OPEN_FILE_KINDS.len()], None), "0 of unknown ()");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_census_counts_this_process() {
        prime();
        let line = census().expect("census on Linux");
        let total: usize = line.split(' ').next().unwrap().parse().unwrap();
        assert!(total >= 3, "{line}");
    }
}
