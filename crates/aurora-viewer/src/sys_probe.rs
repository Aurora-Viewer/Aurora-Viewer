//! What the system knows of the process (AURORA_PROFILE, Windows only):
//! its memory, and the CPU time of each of its threads, gathered by group
//! of threads (the decode pool, the frame workers, the network runtime…).
//! The threads are read by a thread of their own every 10 s, which writes
//! the `cpu summary` line: listing the system's threads takes a few
//! milliseconds that the frame must not pay. Elsewhere nothing is read and
//! no line is written. docs/TESTING.md#profiling lists the fields.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::time::{Duration, Instant};

/// Time covered by one `cpu summary` line.
const PERIOD: Duration = Duration::from_secs(10);

/// The process's memory as the system counts it (bytes).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProcessMemory {
    pub working_set: u64,
    pub peak_working_set: u64,
    /// Committed memory of the process alone (heap, stacks, mapped copies).
    pub private: u64,
    /// Page faults since the start (soft ones included).
    pub page_faults: u32,
}

/// The calling thread's system id (the main thread's is noted at start).
pub fn current_thread_id() -> u32 {
    #[cfg(windows)]
    {
        // SAFETY: no argument, no state changed.
        unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() }
    }
    #[cfg(not(windows))]
    0
}

/// The process's memory now (None where it is not known).
pub fn process_memory() -> Option<ProcessMemory> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        // SAFETY: plain-old-data structure, all zeroes is a valid value.
        let mut c: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
        c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        // SAFETY: the pseudo-handle of the current process is always
        // valid; the call writes at most `cb` bytes into `c`, whose
        // extended layout starts with PROCESS_MEMORY_COUNTERS.
        let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), (&raw mut c).cast::<PROCESS_MEMORY_COUNTERS>(), c.cb) };
        (ok != 0).then_some(ProcessMemory {
            working_set: c.WorkingSetSize as u64,
            peak_working_set: c.PeakWorkingSetSize as u64,
            private: c.PrivateUsage as u64,
            page_faults: c.PageFaultCount,
        })
    }
    #[cfg(not(windows))]
    None
}

/// One thread of the process: its name (empty when it has none) and the
/// processor cycles it has run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadTime {
    pub id: u32,
    pub name: String,
    pub cycles: u64,
}

/// Every thread of the process now, and the cycles of the whole process
/// (those of finished threads included); nothing where unknown.
#[cfg(windows)]
fn read_threads() -> (Vec<ThreadTime>, u64) {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE, LocalFree};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetCurrentProcessId, GetThreadDescription, OpenThread, THREAD_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::System::WindowsProgramming::{QueryProcessCycleTime, QueryThreadCycleTime};
    let mut out = Vec::new();
    let mut process = 0u64;
    // SAFETY: every handle opened here is checked and closed before the
    // function returns; the structures handed to the calls are sized as
    // they expect; the description string the system allocates is read up
    // to its terminating zero and then freed with LocalFree, as
    // GetThreadDescription asks.
    unsafe {
        QueryProcessCycleTime(GetCurrentProcess(), &mut process);
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snapshot == INVALID_HANDLE_VALUE || snapshot.is_null() {
            return (out, process);
        }
        let pid = GetCurrentProcessId();
        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        let mut more = Thread32First(snapshot, &mut entry) != 0;
        while more {
            if entry.th32OwnerProcessID == pid {
                let handle = OpenThread(THREAD_QUERY_LIMITED_INFORMATION, 0, entry.th32ThreadID);
                if !handle.is_null() {
                    let mut cycles = 0u64;
                    QueryThreadCycleTime(handle, &mut cycles);
                    let mut text: windows_sys::core::PWSTR = std::ptr::null_mut();
                    let mut name = String::new();
                    if GetThreadDescription(handle, &mut text) >= 0 && !text.is_null() {
                        let len = (0..).take_while(|&i| *text.add(i) != 0).count();
                        name = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
                        LocalFree(text.cast());
                    }
                    CloseHandle(handle);
                    out.push(ThreadTime {
                        id: entry.th32ThreadID,
                        name,
                        cycles,
                    });
                }
            }
            more = Thread32Next(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
    }
    (out, process)
}

/// The share of the machine's cores busy (all processes), from two
/// readings of the system times: (idle, total) in 100 ns.
#[cfg(windows)]
fn system_times() -> Option<(u64, u64)> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::GetSystemTimes;
    let t = |f: FILETIME| (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime);
    let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
    // SAFETY: the call writes three FILETIME values.
    let ok = unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) };
    // (kernel time includes idle time)
    (ok != 0).then(|| (t(idle), t(kernel) + t(user)))
}

/// The group of a thread: its name without the number of its pool
/// (`aurora-jobs-3` → `aurora-jobs`), `main` for the main thread, `unnamed`
/// for threads without a name (the driver's, the system's).
pub fn group_of(name: &str, id: u32, main: u32) -> String {
    if id == main {
        return "main".to_string();
    }
    if name.is_empty() {
        return "unnamed".to_string();
    }
    let trimmed = name.trim_end_matches(|c: char| c.is_ascii_digit());
    match trimmed.strip_suffix('-') {
        Some(base) if trimmed.len() < name.len() && !base.is_empty() => base.to_string(),
        _ => name.to_string(),
    }
}

/// One group of threads over a period: its CPU time, its threads, and the
/// CPU time of its busiest thread (ms).
#[derive(Debug, Clone, PartialEq)]
pub struct GroupTime {
    pub group: String,
    pub ms: f64,
    pub threads: usize,
    pub busiest_ms: f64,
}

/// CPU time of each group between two readings of the threads, the
/// busiest group first. A thread that started in between counts all its
/// cycles; one that ended is missing (its last cycles are in the process
/// total).
pub fn group_times(before: &HashMap<u32, u64>, now: &[ThreadTime], main: u32, cycles_per_ms: f64) -> Vec<GroupTime> {
    let mut groups: Vec<GroupTime> = Vec::new();
    for t in now {
        let ran = t.cycles.saturating_sub(before.get(&t.id).copied().unwrap_or(0));
        let ms = ran as f64 / cycles_per_ms.max(1.0);
        let group = group_of(&t.name, t.id, main);
        match groups.iter_mut().find(|g| g.group == group) {
            Some(g) => {
                g.ms += ms;
                g.threads += 1;
                g.busiest_ms = g.busiest_ms.max(ms);
            }
            None => groups.push(GroupTime {
                group,
                ms,
                threads: 1,
                busiest_ms: ms,
            }),
        }
    }
    groups.sort_by(|a, b| b.ms.total_cmp(&a.ms));
    groups
}

/// The `cpu summary` line: the process's CPU time as a share of one core
/// and of the machine, the machine's whole load when known, then each
/// group as a share of one core (`threads`, and the busiest thread's share
/// for a pool).
pub fn cpu_line(groups: &[GroupTime], process_ms: f64, elapsed_ms: f64, cores: usize, system_busy: Option<f64>) -> String {
    let pct = |ms: f64| ms / elapsed_ms.max(1.0) * 100.0;
    let mut out = format!(
        "process={:.0}% of one core, {:.1}% of {cores} cores",
        pct(process_ms),
        pct(process_ms) / cores.max(1) as f64
    );
    if let Some(busy) = system_busy {
        let _ = write!(out, " | machine={busy:.1}%");
    }
    out.push_str(" |");
    // (the groups that stayed idle are only counted: the line stays short)
    let mut quiet = 0;
    for g in groups {
        if pct(g.ms) < QUIET_PERCENT && g.group != "main" {
            quiet += 1;
            continue;
        }
        let _ = write!(out, " {}={:.1}%", g.group, pct(g.ms));
        if g.threads > 1 {
            let _ = write!(out, "({}t max={:.1}%)", g.threads, pct(g.busiest_ms));
        }
    }
    if quiet > 0 {
        let _ = write!(out, " | quiet_groups={quiet}");
    }
    out
}

/// Groups below this share of one core are only counted (`quiet_groups`).
const QUIET_PERCENT: f64 = 0.1;

/// Starts the thread that writes `cpu summary` every 10 s; `main` is the
/// main thread's id. Nothing where threads cannot be read.
pub fn start(main: u32) {
    #[cfg(windows)]
    {
        let spawned = std::thread::Builder::new().name("aurora-probe".into()).spawn(move || {
            let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
            let rate = aurora_render::thread_cpu::cycles_per_ms();
            let (threads, mut process) = read_threads();
            let mut before: HashMap<u32, u64> = threads.iter().map(|t| (t.id, t.cycles)).collect();
            let mut system = system_times();
            let mut at = Instant::now();
            loop {
                std::thread::sleep(PERIOD);
                let (threads, process_now) = read_threads();
                let system_now = system_times();
                let now = Instant::now();
                let elapsed_ms = now.duration_since(at).as_secs_f64() * 1000.0;
                let groups = group_times(&before, &threads, main, rate);
                let busy = match (system, system_now) {
                    (Some((idle0, total0)), Some((idle1, total1))) if total1 > total0 => {
                        let total = (total1 - total0) as f64;
                        Some((1.0 - (idle1.saturating_sub(idle0)) as f64 / total) * 100.0)
                    }
                    _ => None,
                };
                let process_ms = process_now.saturating_sub(process) as f64 / rate.max(1.0);
                log::info!("cpu summary: {}", cpu_line(&groups, process_ms, elapsed_ms, cores, busy));
                before = threads.iter().map(|t| (t.id, t.cycles)).collect();
                process = process_now;
                system = system_now;
                at = now;
            }
        });
        if let Err(e) = spawned {
            log::warn!("cpu probe not started: {e}");
        }
    }
    #[cfg(not(windows))]
    let _ = main;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threads_are_grouped_by_pool() {
        assert_eq!(group_of("aurora-jobs-12", 5, 1), "aurora-jobs");
        assert_eq!(group_of("aurora-worker-0", 5, 1), "aurora-worker");
        assert_eq!(group_of("aurora-render", 5, 1), "aurora-render");
        assert_eq!(group_of("", 1, 1), "main");
        assert_eq!(group_of("", 5, 1), "unnamed");
        // a name that is only digits, or without a dash, stays whole
        assert_eq!(group_of("123", 5, 1), "123");
        assert_eq!(group_of("worker7", 5, 1), "worker7");
    }

    #[test]
    fn the_cpu_line_gives_each_group_as_a_share_of_one_core() {
        let t = |id, name: &str, cycles| ThreadTime {
            id,
            name: name.to_string(),
            cycles,
        };
        let before: HashMap<u32, u64> = [(1, 1000), (2, 0), (3, 0)].into_iter().collect();
        // 1 cycle a millisecond: main ran 9 s of 10, two jobs threads 5 s
        // and 2 s, a new thread 1 s
        let now = [
            t(1, "", 10_000),
            t(2, "aurora-jobs-0", 5000),
            t(3, "aurora-jobs-1", 2000),
            t(4, "aurora-net", 1000),
            t(5, "aurora-trash", 5),
        ];
        let groups = group_times(&before, &now, 1, 1.0);
        assert_eq!(
            groups.iter().map(|g| g.group.as_str()).collect::<Vec<_>>(),
            ["main", "aurora-jobs", "aurora-net", "aurora-trash"]
        );
        assert_eq!((groups[1].threads, groups[1].ms, groups[1].busiest_ms), (2, 7000.0, 5000.0));
        let line = cpu_line(&groups, 17_000.0, 10_000.0, 8, Some(40.0));
        assert_eq!(
            line,
            "process=170% of one core, 21.2% of 8 cores | machine=40.0% | main=90.0% aurora-jobs=70.0%(2t max=50.0%) aurora-net=10.0% \
             | quiet_groups=1"
        );
    }

    #[test]
    fn the_process_memory_is_known_on_windows() {
        let m = process_memory();
        if cfg!(windows) {
            let m = m.expect("read");
            assert!(m.working_set > 0 && m.private > 0 && m.peak_working_set >= m.working_set);
        }
    }
}
