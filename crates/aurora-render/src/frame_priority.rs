//! Priority of the threads a frame waits for: the main thread, the render
//! thread and the pool workers that help the main thread in the frame's
//! parallel loops (frame_par.rs).
//!
//! At normal priority they share the cores with every other program's
//! threads: while a compiler, a browser or another viewer keeps the cores
//! busy, a step of the frame loses its core for a scheduler quantum or
//! more, and a helper preempted in the middle of its chunk holds the main
//! thread with it. The worst frames of the audit of 2026-10-13 were of that
//! kind (`lists`, `r_tessellate`, `sync`, `poses` together, almost entirely
//! off the processor: a 27 ms frame); in the demo on a loaded machine,
//! seconds whose worst frame is 30 to 120 ms with every step off the
//! processor. One step above normal, these threads get a core first when
//! they have work; they sleep between frames (frame limiter, wait for the
//! packet, workers without a loop), so they take nothing from the others
//! then. The background decodes stay below normal (scene/jobs.rs).
//!
//! The wanted priority is global; each thread applies it to itself at its
//! next turn ([`apply`]), so it can change while the viewer runs
//! (AURORA_FRAME_PRIORITY=alt compares both, second after second).

use std::cell::Cell;
use std::sync::atomic::{AtomicU8, Ordering};

const NORMAL: u8 = 0;
const RAISED: u8 = 1;
/// Not applied yet on this thread.
const UNSET: u8 = u8::MAX;

static WANTED: AtomicU8 = AtomicU8::new(RAISED);

thread_local! {
    static APPLIED: Cell<u8> = const { Cell::new(UNSET) };
}

/// Raise (or not) the frame's threads, from their next turn on.
pub fn set_raised(raised: bool) {
    #[cfg(target_os = "linux")]
    linux::base_nice();
    WANTED.store(if raised { RAISED } else { NORMAL }, Ordering::Relaxed);
}

pub fn raised() -> bool {
    WANTED.load(Ordering::Relaxed) == RAISED
}

/// Give the calling thread, one of the frame's, the wanted priority if it
/// does not have it yet (a load and a compare when it has).
pub fn apply() {
    let wanted = WANTED.load(Ordering::Relaxed);
    if APPLIED.with(Cell::get) == wanted {
        return;
    }
    APPLIED.with(|a| a.set(wanted));
    set_thread_priority(wanted == RAISED);
}

fn set_thread_priority(raised: bool) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL, THREAD_PRIORITY_NORMAL,
        };
        let priority = if raised {
            THREAD_PRIORITY_ABOVE_NORMAL
        } else {
            THREAD_PRIORITY_NORMAL
        };
        // SAFETY: `GetCurrentThread` returns the calling thread's
        // pseudo-handle, always valid for it; `SetThreadPriority` only
        // reads that handle and an integer.
        unsafe {
            SetThreadPriority(GetCurrentThread(), priority);
        }
    }
    #[cfg(target_os = "linux")]
    linux::set_thread_priority(raised);
    #[cfg(not(any(windows, target_os = "linux")))]
    let _ = raised;
}

/// Linux: the nice value, per thread there. Only the threads of the same
/// scheduling group compete by it: the whole process with the default
/// autogroups (one per session), so a raised frame thread mostly goes
/// ahead of the viewer's own threads, not of another program's.
///
/// Like a Windows thread priority, which is relative to the process's
/// priority class, the values are relative to the nice the viewer was
/// started with (`nice -n 5 aurora-viewer` keeps the order). Going below
/// it needs RLIMIT_NICE (`ulimit -e`, the `gamemode` group grants it on
/// some distributions) or CAP_SYS_NICE: refused once, it is not tried
/// again and the frame's threads stay at the base. Back to normal is
/// allowed: it never goes above the base. A thread inherits the nice of
/// the thread that creates it, so a thread started by a raised one starts
/// raised too (Windows does not do that).
#[cfg(target_os = "linux")]
pub mod linux {
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// One step above normal: nice weights go by ~1.25× a step, as Windows
    /// goes by one level for ABOVE_NORMAL.
    const RAISED_STEP: i32 = 1;

    static BASE: OnceLock<i32> = OnceLock::new();

    /// The nice value the process started with: taken by the main thread
    /// before it raises anything (`set_raised` at startup).
    pub fn base_nice() -> i32 {
        *BASE.get_or_init(thread_nice)
    }

    /// Nice value of the calling thread.
    pub fn thread_nice() -> i32 {
        // SAFETY: `gettid` has no precondition; `getpriority` only reads
        // integers, and cannot fail for the calling thread's own id.
        unsafe { libc::getpriority(libc::PRIO_PROCESS, libc::gettid() as libc::id_t) }
    }

    /// Set the calling thread's nice value; false if refused.
    pub fn set_thread_nice(nice: i32) -> bool {
        // SAFETY: as in `thread_nice`; `setpriority` only reads integers.
        unsafe { libc::setpriority(libc::PRIO_PROCESS, libc::gettid() as libc::id_t, nice.clamp(-20, 19)) == 0 }
    }

    pub(super) fn set_thread_priority(raised: bool) {
        /// Raising was refused once: the process may not go below its base.
        static REFUSED: AtomicBool = AtomicBool::new(false);
        if raised && REFUSED.load(Ordering::Relaxed) {
            return;
        }
        let base = base_nice();
        let ok = set_thread_nice(if raised { base - RAISED_STEP } else { base });
        if !ok && raised && !REFUSED.swap(true, Ordering::Relaxed) {
            log::info!(
                "frame priority: the frame's threads stay at nice {base} ({}); raising them needs RLIMIT_NICE or CAP_SYS_NICE",
                std::io::Error::last_os_error()
            );
        }
    }
}

/// AURORA_FRAME_PRIORITY: `normal` keeps the frame's threads at normal
/// priority, `alt` raises them two seconds in three (comparison); anything
/// else, or nothing, raises them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PriorityMode {
    Raised,
    Normal,
    Alternate,
}

impl PriorityMode {
    pub fn parse(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            Some("normal" | "0" | "off") => PriorityMode::Normal,
            Some("alt") => PriorityMode::Alternate,
            _ => PriorityMode::Raised,
        }
    }

    pub fn from_env() -> Self {
        Self::parse(std::env::var("AURORA_FRAME_PRIORITY").ok().as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mode_is_raised_unless_asked_otherwise() {
        assert_eq!(PriorityMode::parse(None), PriorityMode::Raised);
        assert_eq!(PriorityMode::parse(Some("above")), PriorityMode::Raised);
        assert_eq!(PriorityMode::parse(Some(" normal ")), PriorityMode::Normal);
        assert_eq!(PriorityMode::parse(Some("off")), PriorityMode::Normal);
        assert_eq!(PriorityMode::parse(Some("alt")), PriorityMode::Alternate);
    }

    #[test]
    fn a_thread_applies_the_wanted_priority_once_per_change() {
        // (in a thread of its own: the wanted priority is global, the
        // applied one per thread)
        std::thread::spawn(|| {
            set_raised(true);
            apply();
            assert_eq!(APPLIED.with(Cell::get), RAISED);
            set_raised(false);
            apply();
            assert_eq!(APPLIED.with(Cell::get), NORMAL);
            set_raised(true);
            assert!(raised());
        })
        .join()
        .expect("thread");
    }
}
