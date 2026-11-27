//! The open-file limit on Linux and macOS (#2970). A desktop session starts
//! programs with a soft limit of 1024 open files (macOS: 256), far below the
//! hard limit. Aurora holds a file descriptor for every connection, cache
//! file and GPU synchronisation object (the Vulkan drivers export sync files
//! and DMA buffers as descriptors), and a busy region after a teleport takes
//! it past 1024. Mesa then reports the refused descriptor as "out of
//! memory", and wgpu loses the device: the viewer crashed with "GPU device
//! lost" while the card had plenty of memory left.
//!
//! At startup the soft limit is raised to the hard limit, as browsers and
//! game engines do (systemd's recommendation for programs that do not use
//! `select()`; Aurora's sockets go through epoll / kqueue). On macOS the
//! kernel refuses a soft limit above `OPEN_MAX` even when the hard limit is
//! "unlimited", so it stops there. Nothing changes on Windows, which has no
//! such limit for handles.

/// Highest soft limit macOS accepts for a process (`OPEN_MAX`,
/// sys/syslimits.h), whatever its hard limit.
#[cfg(any(target_os = "macos", test))]
const MACOS_OPEN_MAX: u64 = 10_240;

/// The soft limit to ask for, from the current soft and hard limits, or
/// `None` when the soft limit is already as high as allowed. `cap` is the
/// system's own ceiling (macOS), if any. `u64::MAX` stands for "unlimited".
#[cfg_attr(not(unix), allow(dead_code))]
fn raised(soft: u64, hard: u64, cap: Option<u64>) -> Option<u64> {
    let target = cap.map_or(hard, |cap| hard.min(cap));
    (target > soft).then_some(target)
}

/// Raises the soft open-file limit to the hard one and logs both. Called once
/// at startup, after the log is open and before the first connection or the
/// renderer.
#[cfg(unix)]
pub fn raise() {
    let mut limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    // SAFETY: `&mut limit` is a valid, writable `rlimit` on the stack, and
    // getrlimit writes only that struct.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
        log::warn!("open files: limit unknown ({})", std::io::Error::last_os_error());
        return;
    }
    // rlim_t is u64 on Linux and macOS, the two Unix systems Aurora builds for
    let (soft, hard): (u64, u64) = (limit.rlim_cur, limit.rlim_max);
    #[cfg(target_os = "macos")]
    let cap = Some(MACOS_OPEN_MAX);
    #[cfg(not(target_os = "macos"))]
    let cap = None;
    let Some(target) = raised(soft, hard, cap) else {
        log::info!("open files: soft limit {} (hard {})", shown(soft), shown(hard));
        return;
    };
    let wanted = libc::rlimit {
        rlim_cur: target,
        rlim_max: limit.rlim_max,
    };
    // SAFETY: `&wanted` is a valid `rlimit` on the stack, and setrlimit only
    // reads that struct.
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &wanted) } == 0 {
        log::info!(
            "open files: soft limit raised from {} to {} (hard {})",
            shown(soft),
            shown(target),
            shown(hard)
        );
    } else {
        log::warn!(
            "open files: soft limit stays {} (hard {}): {}",
            shown(soft),
            shown(hard),
            std::io::Error::last_os_error()
        );
    }
}

#[cfg(not(unix))]
pub fn raise() {}

/// A limit for the log, "unlimited" for `RLIM_INFINITY`.
#[cfg(unix)]
fn shown(limit: u64) -> String {
    if limit == libc::RLIM_INFINITY {
        "unlimited".to_owned()
    } else {
        limit.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_desktop_soft_limit_rises_to_the_hard_one() {
        assert_eq!(raised(1024, 524_288, None), Some(524_288));
    }

    #[test]
    fn a_soft_limit_already_at_the_hard_one_stays() {
        assert_eq!(raised(524_288, 524_288, None), None);
        assert_eq!(raised(4096, 4096, None), None);
    }

    #[test]
    fn macos_stops_at_open_max_even_when_unlimited() {
        assert_eq!(raised(256, u64::MAX, Some(MACOS_OPEN_MAX)), Some(MACOS_OPEN_MAX));
        assert_eq!(raised(256, 2048, Some(MACOS_OPEN_MAX)), Some(2048));
        assert_eq!(raised(MACOS_OPEN_MAX, u64::MAX, Some(MACOS_OPEN_MAX)), None);
    }
}
