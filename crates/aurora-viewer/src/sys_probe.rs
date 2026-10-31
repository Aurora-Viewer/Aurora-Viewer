//! What the system knows of the process (AURORA_PROFILE and the resource
//! monitor, Windows and Linux):
//! its memory, and the CPU time of each of its threads, gathered by group
//! of threads (the decode pool, the frame workers, the network runtime…).
//! Linux reads it from `/proc` ([`proc_text`] parses it, on every system for
//! the tests).
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
    #[cfg(target_os = "linux")]
    {
        // SAFETY: no argument, no state changed.
        unsafe { libc::gettid() as u32 }
    }
    #[cfg(not(any(windows, target_os = "linux")))]
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
    #[cfg(target_os = "linux")]
    {
        let status = proc_text::parse_status(&std::fs::read_to_string("/proc/self/status").ok()?)?;
        // SAFETY: plain-old-data structure, all zeroes is a valid value.
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        // SAFETY: the call writes one rusage for the calling process.
        let faults = if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } == 0 {
            (usage.ru_minflt as u64).wrapping_add(usage.ru_majflt as u64)
        } else {
            0
        };
        Some(ProcessMemory {
            working_set: status.rss,
            peak_working_set: status.peak_rss,
            // Windows' private bytes count what is committed whether resident
            // or not: here the anonymous memory, resident or swapped out
            private: status.anon + status.swap,
            // (the counter wraps as Windows' does: readers take differences)
            page_faults: faults as u32,
        })
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    None
}

/// One thread of the process: its name (empty when it has none) and the
/// processor cycles it has run (nanoseconds on Linux, as
/// `thread_cpu::cycles`).
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

/// Every thread of the process now, and the CPU time of the whole process
/// (that of finished threads included), in nanoseconds; nothing where
/// unknown. One thread's files gone between the listing and the read means
/// it ended: it is left out.
#[cfg(target_os = "linux")]
fn read_threads() -> (Vec<ThreadTime>, u64) {
    let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: the call writes one timespec; the process's clock always exists.
    let process = if unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut t) } == 0 {
        (t.tv_sec as u64).wrapping_mul(1_000_000_000).wrapping_add(t.tv_nsec as u64)
    } else {
        0
    };
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir("/proc/self/task") else {
        return (out, process);
    };
    let ns_per_tick = 1e9 / clock_ticks();
    // a thread nobody named carries the name of the thread that started it:
    // the process's when that was the main thread (the driver's threads, the
    // system's), then unnamed, as on Windows; one started by a named thread
    // counts in that thread's group
    let process_name = std::fs::read_to_string("/proc/self/comm").unwrap_or_default();
    for entry in dir.flatten() {
        let Some(id) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else {
            continue;
        };
        let path = entry.path();
        let Some(stat) = std::fs::read_to_string(path.join("stat"))
            .ok()
            .and_then(|t| proc_text::parse_stat(&t))
        else {
            continue;
        };
        // the scheduler's own count is in nanoseconds; the ticks of `stat`
        // (10 ms) are too coarse for half a second, and only a fallback
        let cycles = std::fs::read_to_string(path.join("schedstat"))
            .ok()
            .and_then(|t| proc_text::parse_schedstat(&t))
            .unwrap_or(((stat.utime + stat.stime) as f64 * ns_per_tick) as u64);
        let name = if stat.comm == process_name.trim_end() {
            String::new()
        } else {
            proc_text::full_thread_name(&stat.comm).to_string()
        };
        out.push(ThreadTime { id, name, cycles });
    }
    (out, process)
}

/// The machine's (idle, total) times, in clock ticks.
#[cfg(target_os = "linux")]
fn system_times() -> Option<(u64, u64)> {
    let text = std::fs::read_to_string("/proc/stat").ok()?;
    proc_text::parse_proc_stat(&text).map(|(machine, _)| machine)
}

/// Clock ticks of a second, the unit of `/proc`'s CPU times.
#[cfg(target_os = "linux")]
fn clock_ticks() -> f64 {
    // SAFETY: no pointer, no state changed.
    let n = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if n > 0 { n as f64 } else { 100.0 }
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
    #[cfg(any(windows, target_os = "linux"))]
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
    #[cfg(not(any(windows, target_os = "linux")))]
    let _ = main;
}

// ------------------------------------------------------- resource monitor

/// One group of threads over the last half second (resource monitor).
#[derive(Debug, Clone, PartialEq)]
pub struct GroupLoad {
    pub group: String,
    pub threads: usize,
    /// Share of one core (%), all its threads together.
    pub percent: f32,
    /// Share of one core of its busiest thread (%).
    pub busiest: f32,
    /// Scheduling priority of its threads (`THREAD_PRIORITY_*`), when read.
    pub priority: Option<i32>,
}

/// A process the viewer started (a media plugin, with the processes it
/// started in turn: a web page's renderers) over the last half second.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChildLoad {
    pub pid: u32,
    /// Share of one core (%).
    pub percent: f32,
    pub working_set: u64,
}

/// What the system says of the process and the machine, every half second
/// while the resource monitor is open.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CpuSnapshot {
    /// Logical cores of the machine.
    pub cores: usize,
    /// The process, as a share of the whole machine (%).
    pub process: f32,
    /// The machine's whole load (%), every process together.
    pub machine: Option<f32>,
    /// Load of each logical core (%), when the system's counters answer.
    pub per_core: Vec<f32>,
    /// Groups of threads, the busiest first.
    pub groups: Vec<GroupLoad>,
    pub children: Vec<ChildLoad>,
}

/// How often the monitor's sampler reads the system.
pub const LIVE_PERIOD: Duration = Duration::from_millis(500);

struct Live {
    wanted: std::sync::atomic::AtomicBool,
    main: std::sync::atomic::AtomicU32,
    children: parking_lot::Mutex<Vec<u32>>,
    last: parking_lot::Mutex<Option<CpuSnapshot>>,
}

static LIVE: Live = Live {
    wanted: std::sync::atomic::AtomicBool::new(false),
    main: std::sync::atomic::AtomicU32::new(0),
    children: parking_lot::Mutex::new(Vec::new()),
    last: parking_lot::Mutex::new(None),
};

/// Samples the system every half second while `on` (the resource monitor
/// is open; `main` is the main thread's id), on a thread of its own started
/// the first time: listing the threads costs a few milliseconds the frame
/// must not pay. Off, the thread sleeps and the last snapshot is dropped.
pub fn set_live(on: bool, main: u32) {
    use std::sync::atomic::Ordering::Relaxed;
    LIVE.main.store(main, Relaxed);
    if LIVE.wanted.swap(on, Relaxed) == on {
        return;
    }
    if !on {
        *LIVE.last.lock() = None;
        return;
    }
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let spawned = std::thread::Builder::new().name("aurora-monitor".into()).spawn(live_loop);
        if let Err(e) = spawned {
            log::warn!("resource monitor sampler not started: {e}");
        }
    });
}

/// The processes whose load the sampler reads besides the viewer's (the
/// media plugins).
pub fn watch_children(pids: Vec<u32>) {
    let mut c = LIVE.children.lock();
    if *c != pids {
        *c = pids;
    }
}

/// The last snapshot (None before the first one, or while off).
pub fn live_snapshot() -> Option<CpuSnapshot> {
    LIVE.last.lock().clone()
}

fn live_loop() {
    use std::sync::atomic::Ordering::Relaxed;
    let mut sampler = Sampler::default();
    loop {
        std::thread::sleep(LIVE_PERIOD);
        if !LIVE.wanted.load(Relaxed) {
            sampler = Sampler::default();
            continue;
        }
        let children = LIVE.children.lock().clone();
        if let Some(snapshot) = sampler.sample(LIVE.main.load(Relaxed), &children) {
            *LIVE.last.lock() = Some(snapshot);
        }
    }
}

/// The readings of the previous sample, to turn counters into loads.
#[derive(Default)]
struct Sampler {
    at: Option<Instant>,
    threads: HashMap<u32, u64>,
    process: u64,
    system: Option<(u64, u64)>,
    /// Processes of the watched trees: pid → CPU time (100 ns).
    children: HashMap<u32, u64>,
    #[cfg(windows)]
    cores: Option<CoreCounters>,
    /// Each core's (idle, total) ticks at the previous sample.
    #[cfg(target_os = "linux")]
    cores: Vec<(u64, u64)>,
}

impl Sampler {
    /// The machine's (idle, total) times and each core's load since the
    /// last call.
    #[cfg(windows)]
    fn system_and_cores(&mut self) -> (Option<(u64, u64)>, Vec<f32>) {
        let system = system_times();
        (system, self.cores.get_or_insert_with(CoreCounters::open).read())
    }

    #[cfg(target_os = "linux")]
    fn system_and_cores(&mut self) -> (Option<(u64, u64)>, Vec<f32>) {
        let Some((system, cores)) = std::fs::read_to_string("/proc/stat")
            .ok()
            .and_then(|t| proc_text::parse_proc_stat(&t))
        else {
            return (None, Vec::new());
        };
        let loads = proc_text::core_loads(&self.cores, &cores);
        self.cores = cores;
        (Some(system), loads)
    }

    /// A snapshot from the second sample on.
    #[cfg(any(windows, target_os = "linux"))]
    fn sample(&mut self, main: u32, children: &[u32]) -> Option<CpuSnapshot> {
        let rate = aurora_render::thread_cpu::cycles_per_ms();
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        let (threads, process) = read_threads();
        let (system, per_core) = self.system_and_cores();
        // each watched process with its descendants: (root, pid, time, memory)
        let trees = if children.is_empty() {
            HashMap::new()
        } else {
            process_trees(children)
        };
        let child_times: Vec<(u32, u32, u64, u64)> = trees
            .iter()
            .flat_map(|(&root, pids)| pids.iter().map(move |&pid| (root, pid)))
            .filter_map(|(root, pid)| child_times(pid).map(|(t, ws)| (root, pid, t, ws)))
            .collect();
        let now = Instant::now();
        let out = self.at.map(|at| {
            let elapsed_ms = now.duration_since(at).as_secs_f64() * 1000.0;
            let pct = |ms: f64| (ms / elapsed_ms.max(1.0) * 100.0) as f32;
            let groups = group_times(&self.threads, &threads, main, rate)
                .into_iter()
                .map(|g| GroupLoad {
                    percent: pct(g.ms),
                    busiest: pct(g.busiest_ms),
                    threads: g.threads,
                    priority: group_priority(&threads, &g.group, main),
                    group: g.group,
                })
                .collect();
            let machine = match (self.system, system) {
                (Some((idle0, total0)), Some((idle1, total1))) if total1 > total0 => {
                    Some(((1.0 - idle1.saturating_sub(idle0) as f64 / (total1 - total0) as f64) * 100.0) as f32)
                }
                _ => None,
            };
            let process_ms = process.saturating_sub(self.process) as f64 / rate.max(1.0);
            CpuSnapshot {
                cores,
                process: pct(process_ms) / cores as f32,
                machine,
                per_core,
                groups,
                children: children
                    .iter()
                    .filter(|root| trees.contains_key(root))
                    .map(|&root| {
                        let mut load = ChildLoad {
                            pid: root,
                            percent: 0.0,
                            working_set: 0,
                        };
                        for &(_, pid, t, ws) in child_times.iter().filter(|c| c.0 == root) {
                            // 100 ns units; a process new since the last sample counts from now
                            load.percent += self
                                .children
                                .get(&pid)
                                .map_or(0.0, |&t0| pct(t.saturating_sub(t0) as f64 / 10_000.0));
                            load.working_set += ws;
                        }
                        load
                    })
                    .collect(),
            }
        });
        self.at = Some(now);
        self.threads = threads.iter().map(|t| (t.id, t.cycles)).collect();
        self.process = process;
        self.system = system;
        self.children = child_times.iter().map(|&(_, pid, t, _)| (pid, t)).collect();
        out
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    fn sample(&mut self, _main: u32, _children: &[u32]) -> Option<CpuSnapshot> {
        None
    }
}

/// The priority of the first thread of a group (a pool's threads share
/// theirs).
#[cfg(windows)]
fn group_priority(threads: &[ThreadTime], group: &str, main: u32) -> Option<i32> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{GetThreadPriority, OpenThread, THREAD_QUERY_LIMITED_INFORMATION};
    // (GetThreadPriority's error value, MAXLONG)
    const THREAD_PRIORITY_ERROR_RETURN: i32 = i32::MAX;
    let t = threads.iter().find(|t| group_of(&t.name, t.id, main) == group)?;
    // SAFETY: the handle is checked, read and closed here.
    unsafe {
        let handle = OpenThread(THREAD_QUERY_LIMITED_INFORMATION, 0, t.id);
        if handle.is_null() {
            return None;
        }
        let p = GetThreadPriority(handle);
        CloseHandle(handle);
        (p != THREAD_PRIORITY_ERROR_RETURN).then_some(p)
    }
}

/// The nice value of the first thread of a group (a pool's threads share
/// theirs): -20 to 19, higher gives way (`scene::jobs` lowers its pool by
/// 10).
#[cfg(target_os = "linux")]
fn group_priority(threads: &[ThreadTime], group: &str, main: u32) -> Option<i32> {
    let t = threads.iter().find(|t| group_of(&t.name, t.id, main) == group)?;
    let text = std::fs::read_to_string(format!("/proc/self/task/{}/stat", t.id)).ok()?;
    proc_text::parse_stat(&text).map(|s| s.nice)
}

/// Each of `roots` that still runs, with every process it started
/// (directly or not), itself first.
#[cfg(windows)]
fn process_trees(roots: &[u32]) -> HashMap<u32, Vec<u32>> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    let mut parents: Vec<(u32, u32)> = Vec::new();
    // SAFETY: the snapshot is checked and closed; the entry is sized as the
    // calls expect.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE || snapshot.is_null() {
            return HashMap::new();
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut more = Process32FirstW(snapshot, &mut entry) != 0;
        while more {
            parents.push((entry.th32ProcessID, entry.th32ParentProcessID));
            more = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
    }
    descendants(roots, &parents)
}

#[cfg(target_os = "linux")]
fn process_trees(roots: &[u32]) -> HashMap<u32, Vec<u32>> {
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return HashMap::new();
    };
    let parents: Vec<(u32, u32)> = dir
        .flatten()
        .filter_map(|e| e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()))
        .filter_map(|pid| {
            let stat = proc_text::parse_stat(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)?;
            Some((pid, stat.ppid))
        })
        .collect();
    descendants(roots, &parents)
}

/// From (pid, parent pid) pairs: each root that is listed, with its
/// descendants (a pid reused by an unrelated process after its parent
/// ended can slip in: the system does not tell them apart).
pub fn descendants(roots: &[u32], parents: &[(u32, u32)]) -> HashMap<u32, Vec<u32>> {
    let mut out = HashMap::new();
    for &root in roots {
        if !parents.iter().any(|&(pid, _)| pid == root) {
            continue;
        }
        let mut tree = vec![root];
        let mut i = 0;
        while i < tree.len() && tree.len() < 256 {
            let parent = tree[i];
            for &(pid, _) in parents.iter().filter(|&&(pid, p)| p == parent && pid != parent) {
                if !tree.contains(&pid) {
                    tree.push(pid);
                }
            }
            i += 1;
        }
        out.insert(root, tree);
    }
    out
}

/// A child process's CPU time (100 ns) and working set, None once it has
/// ended or cannot be read.
#[cfg(windows)]
fn child_times(pid: u32) -> Option<(u64, u64)> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ};
    let t = |f: FILETIME| (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime);
    // SAFETY: the handle is checked and closed before returning; the calls
    // write into structures of the size they are given.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, 0, pid);
        if handle.is_null() {
            return None;
        }
        let (mut created, mut exited, mut kernel, mut user) =
            (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let ok = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) != 0;
        let mut m: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        m.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        let mem = K32GetProcessMemoryInfo(handle, &mut m, m.cb) != 0;
        CloseHandle(handle);
        ok.then(|| (t(kernel) + t(user), if mem { m.WorkingSetSize as u64 } else { 0 }))
    }
}

#[cfg(target_os = "linux")]
fn child_times(pid: u32) -> Option<(u64, u64)> {
    let stat = proc_text::parse_stat(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)?;
    // SAFETY: no pointer, no state changed.
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(0) as u64;
    // ticks to the 100 ns of Windows' process times, which the sampler counts in
    let time = ((stat.utime + stat.stime) as f64 * 1e7 / clock_ticks()) as u64;
    Some((time, stat.rss_pages.saturating_mul(page)))
}

/// The system's per-core load counters (`\Processor(*)\% Processor Time`,
/// by their English names whatever the system's language); none when they
/// cannot be opened.
#[cfg(windows)]
struct CoreCounters {
    query: windows_sys::Win32::System::Performance::PDH_HQUERY,
    counter: windows_sys::Win32::System::Performance::PDH_HCOUNTER,
}

// SAFETY: the query handle is only ever used by the sampler's thread, which
// owns the counters.
#[cfg(windows)]
unsafe impl Send for CoreCounters {}

#[cfg(windows)]
impl CoreCounters {
    fn open() -> CoreCounters {
        use windows_sys::Win32::System::Performance::{PdhAddEnglishCounterW, PdhCollectQueryData, PdhOpenQueryW};
        let path: Vec<u16> = "\\Processor(*)\\% Processor Time\0".encode_utf16().collect();
        let mut query = std::ptr::null_mut();
        let mut counter = std::ptr::null_mut();
        // SAFETY: the out pointers are valid; the path ends with a zero.
        unsafe {
            if PdhOpenQueryW(std::ptr::null(), 0, &mut query) != 0 {
                return CoreCounters {
                    query: std::ptr::null_mut(),
                    counter: std::ptr::null_mut(),
                };
            }
            if PdhAddEnglishCounterW(query, path.as_ptr(), 0, &mut counter) != 0 {
                counter = std::ptr::null_mut();
            }
            // the first collection only sets the baseline
            PdhCollectQueryData(query);
        }
        CoreCounters { query, counter }
    }

    /// Load of each logical core since the last read (%), in core order.
    fn read(&mut self) -> Vec<f32> {
        use windows_sys::Win32::System::Performance::{
            PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_MORE_DATA, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
        };
        if self.counter.is_null() {
            return Vec::new();
        }
        let mut cores: Vec<(u32, f32)> = Vec::new();
        // SAFETY: the buffer is sized from the size the first call reports,
        // aligned for the items, and only the `count` items it says it
        // wrote are read; their names point inside that buffer.
        unsafe {
            if PdhCollectQueryData(self.query) != 0 {
                return Vec::new();
            }
            let (mut size, mut count) = (0u32, 0u32);
            if PdhGetFormattedCounterArrayW(self.counter, PDH_FMT_DOUBLE, &mut size, &mut count, std::ptr::null_mut()) != PDH_MORE_DATA {
                return Vec::new();
            }
            let item = std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>();
            let mut buffer: Vec<PDH_FMT_COUNTERVALUE_ITEM_W> = vec![std::mem::zeroed(); (size as usize).div_ceil(item) + 1];
            if PdhGetFormattedCounterArrayW(self.counter, PDH_FMT_DOUBLE, &mut size, &mut count, buffer.as_mut_ptr()) != 0 {
                return Vec::new();
            }
            for it in buffer.iter().take(count as usize) {
                if it.szName.is_null() || it.FmtValue.CStatus != 0 {
                    continue;
                }
                let len = (0..).take_while(|&i| *it.szName.add(i) != 0).count();
                let name = String::from_utf16_lossy(std::slice::from_raw_parts(it.szName, len));
                // "_Total" and anything not a plain core number left out
                if let Ok(n) = name.parse::<u32>() {
                    cores.push((n, it.FmtValue.Anonymous.doubleValue.clamp(0.0, 100.0) as f32));
                }
            }
        }
        cores.sort_by_key(|c| c.0);
        cores.into_iter().map(|c| c.1).collect()
    }
}

#[cfg(windows)]
impl Drop for CoreCounters {
    fn drop(&mut self) {
        if !self.query.is_null() {
            // SAFETY: the query was opened by `open` and is closed once.
            unsafe {
                windows_sys::Win32::System::Performance::PdhCloseQuery(self.query);
            }
        }
    }
}

/// The text of Linux's `/proc` files, parsed. Pure, so that the tests run
/// on every system; nothing here trusts the text: a line missing or
/// malformed gives None, never a panic.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub mod proc_text {
    /// The fields of `/proc/<pid>/stat` (or of a thread's,
    /// `/proc/<pid>/task/<tid>/stat`) the probes read.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Stat {
        /// The name, cut to 15 bytes by the kernel.
        pub comm: String,
        pub ppid: u32,
        /// CPU time in user and system mode, in clock ticks.
        pub utime: u64,
        pub stime: u64,
        pub nice: i32,
        pub rss_pages: u64,
    }

    /// `pid (comm) state ppid …`: the name may hold spaces and brackets,
    /// so the fields are counted from the last `)`.
    pub fn parse_stat(text: &str) -> Option<Stat> {
        let open = text.find('(')?;
        let close = text.rfind(')')?;
        let comm = text.get(open + 1..close)?.to_string();
        // fields from the 3rd (state) on: ppid is the 4th, utime the 14th,
        // stime the 15th, nice the 19th, rss the 24th (proc_pid_stat(5))
        let f: Vec<&str> = text.get(close + 1..)?.split_whitespace().collect();
        let field = |n: usize| f.get(n - 3).copied();
        Some(Stat {
            comm,
            ppid: field(4)?.parse().ok()?,
            utime: field(14)?.parse().ok()?,
            stime: field(15)?.parse().ok()?,
            nice: field(19)?.parse().ok()?,
            rss_pages: field(24)?.parse().ok()?,
        })
    }

    /// `schedstat`: the time on the processor in nanoseconds, its first
    /// field (absent from a kernel built without schedstats).
    pub fn parse_schedstat(text: &str) -> Option<u64> {
        text.split_whitespace().next()?.parse().ok()
    }

    /// The memory lines of `/proc/self/status`, in bytes.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct Status {
        pub rss: u64,
        pub peak_rss: u64,
        pub anon: u64,
        pub swap: u64,
    }

    /// `VmRSS`, `VmHWM`, `RssAnon` and `VmSwap` (in kB); None without
    /// `VmRSS`. A kernel thread or an old kernel may lack the others: 0.
    pub fn parse_status(text: &str) -> Option<Status> {
        let mut out = Status::default();
        let mut rss = None;
        for line in text.lines() {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let Some(kb) = value.split_whitespace().next().and_then(|v| v.parse::<u64>().ok()) else {
                continue;
            };
            let bytes = kb.saturating_mul(1024);
            match key {
                "VmRSS" => rss = Some(bytes),
                "VmHWM" => out.peak_rss = bytes,
                "RssAnon" => out.anon = bytes,
                "VmSwap" => out.swap = bytes,
                _ => {}
            }
        }
        out.rss = rss?;
        out.peak_rss = out.peak_rss.max(out.rss);
        Some(out)
    }

    /// `/proc/stat`: the whole machine's (idle, total) ticks, then each
    /// core's in core order. Idle counts the wait for the disk (`iowait`),
    /// as Windows' idle time does; `guest` is already inside `user`.
    pub fn parse_proc_stat(text: &str) -> Option<((u64, u64), Vec<(u64, u64)>)> {
        let times = |rest: &str| -> Option<(u64, u64)> {
            let v: Vec<u64> = rest.split_whitespace().take(8).map(|x| x.parse().ok()).collect::<Option<_>>()?;
            // user nice system idle iowait irq softirq steal
            (v.len() >= 5).then(|| (v[3] + v[4], v.iter().sum()))
        };
        let mut machine = None;
        let mut cores: Vec<(u32, (u64, u64))> = Vec::new();
        for line in text.lines() {
            let Some(rest) = line.strip_prefix("cpu") else {
                continue;
            };
            if let Some(rest) = rest.strip_prefix(' ') {
                machine = times(rest);
            } else if let Some((n, rest)) = rest.split_once(' ')
                && let Ok(n) = n.parse::<u32>()
                && let Some(t) = times(rest)
            {
                cores.push((n, t));
            }
        }
        cores.sort_by_key(|c| c.0);
        Some((machine?, cores.into_iter().map(|c| c.1).collect()))
    }

    /// Each core's load (%) between two readings of `/proc/stat`; nothing
    /// until there are two, or when the cores changed (one brought online).
    pub fn core_loads(before: &[(u64, u64)], now: &[(u64, u64)]) -> Vec<f32> {
        if before.len() != now.len() {
            return Vec::new();
        }
        before
            .iter()
            .zip(now)
            .map(|(&(idle0, total0), &(idle1, total1))| {
                let total = total1.saturating_sub(total0);
                if total == 0 {
                    return 0.0;
                }
                ((1.0 - idle1.saturating_sub(idle0) as f64 / total as f64) * 100.0).clamp(0.0, 100.0) as f32
            })
            .collect()
    }

    /// The names the viewer gives its threads that the kernel cuts to 15
    /// bytes, as the resource monitor knows them (its roles,
    /// `ui/monitor/threads.rs`, and its attribution, `ui/monitor/model.rs`,
    /// whose test checks that each comes back whole).
    const LONG_NAMES: &[&str] = &[
        "aurora-render-finish",
        "aurora-audio-out",
        "aurora-audio-stream",
        "aurora-inventory-cache",
        "aurora-notification-history",
        "aurora-transcript",
        "aurora-favorites",
        "aurora-web-profile",
        "aurora-web-cache",
        "aurora-hud-scales",
        "aurora-local-previews",
        "aurora-photo-save",
        "aurora-geometry-needs",
        "aurora-hover-height",
    ];

    /// A thread's whole name from the cut one, when the cut one starts
    /// exactly one of [`LONG_NAMES`]. A pool's numbered name cut short
    /// (`aurora-worker-1` for `aurora-worker-12`) keeps its group anyway.
    pub fn full_thread_name(comm: &str) -> &str {
        if comm.len() != 15 {
            return comm;
        }
        let mut found = LONG_NAMES.iter().filter(|n| n.starts_with(comm));
        match (found.next(), found.next()) {
            (Some(n), None) => n,
            _ => comm,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proc_stat_files_are_read_from_their_last_bracket() {
        // captured on Linux 7.2 (a viewer thread, then a name with a space
        // and a bracket in it)
        let line = "4242 (aurora-jobs-3) S 4100 4100 4000 0 -1 4194368 120 0 0 0 731 52 0 0 30 10 61 0 9001 \
                    10485760 2560 18446744073709551615 1 1 0 0 0 0 0 4096 0 0 0 0 -1 7 0 0 0 0 0\n";
        let s = proc_text::parse_stat(line).expect("parsed");
        assert_eq!(
            (s.comm.as_str(), s.ppid, s.utime, s.stime, s.nice, s.rss_pages),
            ("aurora-jobs-3", 4100, 731, 52, 10, 2560)
        );
        let odd = "17 (a) b (c) R 1 17 17 0 -1 0 0 0 0 0 5 6 0 0 20 -3 1 0 0 0 99 0";
        let s = proc_text::parse_stat(odd).expect("parsed");
        assert_eq!((s.comm.as_str(), s.utime, s.stime, s.nice, s.rss_pages), ("a) b (c", 5, 6, -3, 99));
        assert_eq!(proc_text::parse_stat("17 (cut"), None);
        assert_eq!(proc_text::parse_stat("17 (short) R 1 2"), None);
        assert_eq!(proc_text::parse_schedstat("97843117 1593 2\n"), Some(97_843_117));
        assert_eq!(proc_text::parse_schedstat(""), None);
    }

    #[test]
    fn proc_status_gives_the_process_memory() {
        let text = "Name:\taurora-viewer\nVmPeak:\t 9000000 kB\nVmHWM:\t  812340 kB\nVmRSS:\t  700000 kB\n\
                    RssAnon:\t  500000 kB\nRssFile:\t  190000 kB\nVmSwap:\t    1000 kB\nThreads:\t112\n";
        let s = proc_text::parse_status(text).expect("parsed");
        assert_eq!(
            (s.rss, s.peak_rss, s.anon, s.swap),
            (700_000 << 10, 812_340 << 10, 500_000 << 10, 1000 << 10)
        );
        assert_eq!(proc_text::parse_status("Name:\tx\nThreads:\t1\n"), None);
    }

    #[test]
    fn proc_stat_gives_each_core_and_the_machine() {
        let text = "cpu  100 0 50 800 50 0 0 0 0 0\ncpu1 60 0 20 400 20 0 0 0 0 0\ncpu0 40 0 30 400 30 0 0 0 0 0\n\
                    intr 1 2 3\nctxt 99\n";
        let (machine, cores) = proc_text::parse_proc_stat(text).expect("parsed");
        assert_eq!(machine, (850, 1000));
        assert_eq!(cores, [(430, 500), (420, 500)]);
        // core 0 then ran 30 ticks of 100, core 1 all of them
        let later = [(500, 600), (420, 600)];
        assert_eq!(proc_text::core_loads(&cores, &later), [30.0, 100.0]);
        assert!(proc_text::core_loads(&[], &later).is_empty());
        assert_eq!(proc_text::parse_proc_stat("intr 1\n"), None);
    }

    #[test]
    fn thread_names_cut_by_the_kernel_are_made_whole() {
        assert_eq!(proc_text::full_thread_name("aurora-render-f"), "aurora-render-finish");
        assert_eq!(proc_text::full_thread_name("aurora-audio-ou"), "aurora-audio-out");
        assert_eq!(proc_text::full_thread_name("aurora-audio-st"), "aurora-audio-stream");
        assert_eq!(proc_text::full_thread_name("aurora-render"), "aurora-render");
        assert_eq!(group_of(proc_text::full_thread_name("aurora-worker-1"), 9, 1), "aurora-worker");
    }

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
    fn a_plugin_counts_the_processes_it_started() {
        // 10 started 11 and 12, 12 started 13; 20 is someone else's
        let parents = [(10, 1), (11, 10), (12, 10), (13, 12), (20, 1)];
        let trees = descendants(&[10, 99], &parents);
        assert_eq!(trees.len(), 1, "a root that ended is left out");
        assert_eq!(trees[&10], [10, 11, 12, 13]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_sampler_reads_the_threads_and_the_cores_on_linux() {
        let main = current_thread_id();
        let mut sampler = Sampler::default();
        assert_eq!(sampler.sample(main, &[]), None, "the first sample is the baseline");
        // a named thread busy until the sample is taken, and a child process
        let (done, stop) = std::sync::mpsc::channel::<()>();
        let busy = std::thread::Builder::new()
            .name("probe-busy-1".into())
            .spawn(move || {
                while stop.try_recv() == Err(std::sync::mpsc::TryRecvError::Empty) {
                    std::hint::spin_loop();
                }
            })
            .expect("spawned");
        let mut child = std::process::Command::new("sleep").arg("5").spawn().expect("sleep runs");
        std::thread::sleep(Duration::from_millis(100));
        let snapshot = sampler.sample(main, &[child.id()]);
        let _ = done.send(());
        busy.join().expect("joined");
        let _ = child.kill();
        let _ = child.wait();
        let snapshot = snapshot.expect("second sample");
        // (a share of the core, however small on a loaded machine)
        let group = snapshot.groups.iter().find(|g| g.group == "probe-busy").expect("busy group listed");
        assert!(group.percent > 0.0, "{group:?}");
        assert!(snapshot.groups.iter().any(|g| g.group == "main"));
        assert!(!snapshot.per_core.is_empty() && snapshot.machine.is_some());
        assert_eq!(snapshot.children.len(), 1);
        assert!(snapshot.children[0].working_set > 0);
    }

    #[test]
    fn the_process_memory_is_known_on_windows_and_linux() {
        let m = process_memory();
        if cfg!(any(windows, target_os = "linux")) {
            let m = m.expect("read");
            assert!(m.working_set > 0 && m.private > 0 && m.peak_working_set >= m.working_set);
        }
    }
}
