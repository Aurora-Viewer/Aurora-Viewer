//! The native stacks on Linux, written as on Windows (crash/native.rs):
//! `module+0xoffset` with each module's build ID, the only form that can be
//! symbolized after the process is gone.
//!
//! A fatal signal (SIGSEGV, SIGBUS, SIGILL, SIGFPE, SIGABRT) writes its
//! number, its fault and the stack to the pre-opened `native.txt`, then
//! hands the signal on exactly as before: to the standard library's own
//! handler (its stack overflow message) when there was one, else to the
//! default action, which ends the process with that signal. The handler
//! runs on the thread's alternate signal stack when it has one (the
//! standard library gives one to each of its threads), so a stack overflow
//! is written too. It does not allocate, take application locks or log:
//! only `write`, `fsync`, the unwinder primed at install, and the loader's
//! module list, both under an alarm that ends the process should the crash
//! have happened while the loader was busy. Another thread crashing
//! meanwhile waits for the first one's file.
//!
//! Legacy Viewer reference: setupBreakpad (indra/newview/llappviewerlinux.cpp),
//! which writes a minidump on the same signals through Breakpad's
//! ExceptionHandler; Aurora writes the module + offset text its Windows
//! reports already use instead (no minidump, nothing sent by itself).

use std::ffi::{CStr, c_int, c_void};
use std::io::Write;
use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

const FRAMES: usize = 64;
/// The signals that end the process with a crash.
const SIGNALS: [c_int; 5] = [libc::SIGSEGV, libc::SIGBUS, libc::SIGILL, libc::SIGFPE, libc::SIGABRT];
/// Seconds the unwinder and the loader's module list may take inside the
/// handler.
const WRITE_ALARM: u32 = 5;
/// How long another thread that crashes meanwhile waits for the first one's
/// file, in 10 ms naps: longer than [`WRITE_ALARM`].
const OTHER_WAIT_NAPS: u32 = 800;

static TRACE: OnceLock<std::fs::File> = OnceLock::new();
/// The thread writing `native.txt`, 0 before the first fatal signal.
static WRITER: AtomicI32 = AtomicI32::new(0);
/// `native.txt` is complete.
static WRITTEN_TRACE: AtomicBool = AtomicBool::new(false);
static OOM: OnceLock<std::fs::File> = OnceLock::new();
static OOM_WRITING: AtomicBool = AtomicBool::new(false);
/// The handlers found at install, one per [`SIGNALS`], handed the signal on.
static PREVIOUS: [OnceLock<libc::sigaction>; SIGNALS.len()] = [const { OnceLock::new() }; SIGNALS.len()];
/// The executable's file name: the loader lists it without one.
static EXE: OnceLock<Name> = OnceLock::new();

pub fn install(dir: &Path) -> anyhow::Result<()> {
    let file = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("native.txt"))?;
    let _ = TRACE.set(file);
    let oom = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("oom.txt"))?;
    let _ = OOM.set(oom);
    if let Some(name) = std::env::current_exe().ok().as_deref().and_then(Path::file_name) {
        let _ = EXE.set(Name::new(name.as_encoded_bytes()));
    }
    // Once outside any handler: the unwinder and the loader's list resolve
    // their symbols and caches at their first use, which may allocate.
    let mut primer = Vec::new();
    write_stack(&mut primer);
    for (index, &signal) in SIGNALS.iter().enumerate() {
        // SAFETY: plain sigaction calls on zeroed structs we own; the
        // handler only touches static storage and async-signal-safe calls.
        unsafe {
            let mut old: libc::sigaction = std::mem::zeroed();
            anyhow::ensure!(
                libc::sigaction(signal, std::ptr::null(), &mut old) == 0,
                "cannot read signal {signal}"
            );
            let _ = PREVIOUS[index].set(old);
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = on_fatal as extern "C" fn(c_int, *mut libc::siginfo_t, *mut c_void) as libc::sighandler_t;
            action.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
            libc::sigemptyset(&mut action.sa_mask);
            anyhow::ensure!(
                libc::sigaction(signal, &action, std::ptr::null_mut()) == 0,
                "cannot handle signal {signal}"
            );
        }
    }
    install_snapshot()
}

extern "C" fn on_fatal(signal: c_int, info: *mut libc::siginfo_t, context: *mut c_void) {
    // SAFETY: no argument; async-signal-safe.
    let me = unsafe { libc::gettid() };
    match WRITER.compare_exchange(0, me, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => {
            if let Some(file) = TRACE.get() {
                // SAFETY: the kernel's records for this handler, handed on unchanged.
                unsafe { write_signal(file, signal, info, context) };
            }
            WRITTEN_TRACE.store(true, Ordering::Release);
        }
        // Another thread crashing meanwhile (a corrupted heap often takes
        // several down) waits for the first one's file: handed on now, its
        // signal would end the process in the middle of it. The first
        // one's own end, or the alarm, ends this one too.
        Err(writer) if writer != me => {
            let nap = libc::timespec {
                tv_sec: 0,
                tv_nsec: 10_000_000,
            };
            for _ in 0..OTHER_WAIT_NAPS {
                if WRITTEN_TRACE.load(Ordering::Acquire) {
                    break;
                }
                // SAFETY: nanosleep is async-signal-safe; the timespec is ours.
                unsafe { libc::nanosleep(&nap, std::ptr::null_mut()) };
            }
        }
        // The writing thread itself again: the standard library's overflow
        // handler, handed the signal, aborts.
        Err(_) => {}
    }
    // SAFETY: the arguments are the ones the kernel gave this handler.
    unsafe { hand_on(signal, info, context) };
}

/// The signal, its fault and the stack, raw then by module. Apart from the
/// handler so that its arrays are off the alternate stack (8 KB) before the
/// signal is handed on: the standard library's overflow handler then aborts,
/// and SIGABRT comes back here on what is left of it.
///
/// # Safety
/// `info` and `context` are null or the live records of an SA_SIGINFO handler.
#[inline(never)]
unsafe fn write_signal(mut file: &std::fs::File, signal: c_int, info: *const libc::siginfo_t, context: *const c_void) {
    // SAFETY: see the function's contract; both are checked for null.
    let (code, address, pc) = unsafe { fault(info, context) };
    let _ = writeln!(
        file,
        "Native signal {signal} ({})\nSignal code: {code}",
        signal_name(signal).unwrap_or("?")
    );
    // Only a fault raised by the processor has an address; a sent signal
    // (abort, kill) carries the sender there.
    if code > 0 {
        let _ = writeln!(file, "Fault address: {address:#x}");
    }
    let _ = writeln!(file, "Native stack (addresses):");
    let _ = file.sync_all();
    // The unwinder and the module list both go through the loader's list,
    // under its lock: should the crash have happened while another thread
    // held it (loading a library), the alarm's default action ends the
    // process instead of a hang.
    // SAFETY: alarm is async-signal-safe.
    unsafe { libc::alarm(WRITE_ALARM) };
    let mut frames = [0usize; FRAMES];
    let count = capture(&mut frames);
    for (index, address) in frames[..count].iter().enumerate() {
        let _ = writeln!(file, "{index:02}: {address:#x}");
    }
    // The module headers are read once the raw stack is on disk.
    let _ = file.sync_all();
    write_modules(&mut file, &frames[..count], pc);
    let _ = file.sync_all();
    // SAFETY: as above.
    unsafe { libc::alarm(0) };
}

/// The signal code, the faulting address and the faulting instruction.
///
/// # Safety
/// `info` and `context` are null or the live records of an SA_SIGINFO handler.
unsafe fn fault(info: *const libc::siginfo_t, context: *const c_void) -> (c_int, usize, Option<usize>) {
    // SAFETY: checked for null; si_addr is meaningful for the fault signals
    // and harmless to read for the others.
    let (code, address) = unsafe { info.as_ref().map_or((0, 0), |info| (info.si_code, info.si_addr() as usize)) };
    let context = context.cast::<libc::ucontext_t>();
    // SAFETY: checked for null; the machine context is the interrupted
    // thread's registers.
    #[cfg(target_arch = "x86_64")]
    let pc = unsafe { context.as_ref().map(|c| c.uc_mcontext.gregs[libc::REG_RIP as usize] as usize) };
    // SAFETY: as above.
    #[cfg(target_arch = "aarch64")]
    let pc = unsafe { context.as_ref().map(|c| c.uc_mcontext.pc as usize) };
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    let pc = {
        let _ = context;
        None
    };
    (code, address, pc.filter(|&pc| pc != 0))
}

/// Gives the signal the handling it had before ours: the standard
/// library's handler when there was one (it prints the stack overflow
/// message and aborts, or restores the default action and returns), else
/// the default action. A fault returns into the faulting instruction, which
/// faults again under it; a signal that was sent (abort, kill) is raised
/// again.
///
/// # Safety
/// The arguments are those of a running SA_SIGINFO handler for `signal`.
unsafe fn hand_on(signal: c_int, info: *mut libc::siginfo_t, context: *mut c_void) {
    let previous = SIGNALS.iter().position(|&s| s == signal).and_then(|index| PREVIOUS[index].get());
    // SAFETY: plain sigaction calls on structs we own; the previous handler
    // is called with the arguments the kernel gave ours, with the ABI its
    // flags declare.
    unsafe {
        let mut default: libc::sigaction = std::mem::zeroed();
        default.sa_sigaction = libc::SIG_DFL;
        let handler = match previous {
            Some(previous) if previous.sa_sigaction != libc::SIG_IGN => {
                libc::sigaction(signal, previous, std::ptr::null_mut());
                previous.sa_sigaction
            }
            _ => {
                libc::sigaction(signal, &default, std::ptr::null_mut());
                libc::SIG_DFL
            }
        };
        if handler != libc::SIG_DFL {
            if previous.is_some_and(|p| p.sa_flags & libc::SA_SIGINFO != 0) {
                let handler: extern "C" fn(c_int, *mut libc::siginfo_t, *mut c_void) = std::mem::transmute(handler);
                handler(signal, info, context);
            } else {
                let handler: extern "C" fn(c_int) = std::mem::transmute(handler);
                handler(signal);
            }
        }
        if info.as_ref().is_none_or(|info| info.si_code <= 0) {
            // Sent rather than raised by the processor: nothing faults again.
            libc::sigaction(signal, &default, std::ptr::null_mut());
            libc::raise(signal);
        }
    }
}

/// The usual name of a fatal signal.
pub(super) fn signal_name(signal: c_int) -> Option<&'static str> {
    Some(match signal {
        libc::SIGSEGV => "SIGSEGV",
        libc::SIGBUS => "SIGBUS",
        libc::SIGILL => "SIGILL",
        libc::SIGFPE => "SIGFPE",
        libc::SIGABRT => "SIGABRT",
        _ => return None,
    })
}

/// From the allocator, when an allocation fails: its size and the failing
/// thread's stack, before the standard library aborts. No allocation: the
/// file was opened at start and the stack goes through fixed arrays.
pub fn out_of_memory(size: usize) {
    if let Some(mut file) = OOM.get()
        && !OOM_WRITING.swap(true, Ordering::Relaxed)
    {
        let _ = writeln!(file, "Out of memory: {size} bytes");
        write_stack(&mut file);
        let _ = file.sync_all();
        OOM_WRITING.store(false, Ordering::Relaxed);
    }
}

/// The current thread's stack as module + offset, for a Rust panic.
pub fn write_stack(out: &mut impl Write) {
    let mut frames = [0usize; FRAMES];
    let count = capture(&mut frames);
    // The first frame is `capture`'s caller, this function.
    write_modules(out, frames[..count].get(1..).unwrap_or(&[]), None);
}

/// The calling thread's return addresses, through the unwinder the
/// standard library links (libgcc_s), into a fixed array. From a signal
/// handler it walks through the kernel's signal frame into the interrupted
/// code; the frame that faulted is its exact instruction. Never inlined:
/// its own frame is the first one, dropped.
#[inline(never)]
fn capture(frames: &mut [usize; FRAMES]) -> usize {
    struct Walk<'a> {
        frames: &'a mut [usize; FRAMES],
        count: usize,
    }
    extern "C" fn step(context: *mut c_void, walk: *mut c_void) -> c_int {
        // SAFETY: `walk` is the Walk below, alive for the whole backtrace;
        // the context is the unwinder's own for this callback.
        let walk = unsafe { &mut *walk.cast::<Walk>() };
        if walk.count == FRAMES {
            return URC_END_OF_STACK;
        }
        // SAFETY: as above.
        let ip = unsafe { _Unwind_GetIP(context) };
        if ip == 0 {
            return URC_END_OF_STACK;
        }
        walk.frames[walk.count] = ip;
        walk.count += 1;
        URC_NO_REASON
    }
    let mut walk = Walk { frames, count: 0 };
    // SAFETY: the callback only writes inside the fixed array it is given.
    unsafe { _Unwind_Backtrace(step, (&raw mut walk).cast()) };
    // The first frame is this function.
    let count = walk.count;
    if count > 0 {
        walk.frames.copy_within(1..count, 0);
    }
    count.saturating_sub(1)
}

const URC_NO_REASON: c_int = 0;
const URC_END_OF_STACK: c_int = 5;

unsafe extern "C" {
    fn _Unwind_Backtrace(trace: extern "C" fn(*mut c_void, *mut c_void) -> c_int, argument: *mut c_void) -> c_int;
    fn _Unwind_GetIP(context: *mut c_void) -> usize;
}

/// `name+0xoffset` per frame, the offset being the ELF virtual address
/// (what `addr2line -e <file>` and the symbol files read), then each
/// module's identity. Writes nothing but `out`.
fn write_modules(out: &mut impl Write, frames: &[usize], fault: Option<usize>) {
    // Load biases only: this may run on an alternate signal stack of 8 KB,
    // so each module is looked up again for its identity at the end.
    let mut bases = [0usize; FRAMES + 1];
    let mut known = 0;
    let mut location = |out: &mut dyn Write, address: usize| match find(address) {
        Some(module) => {
            let _ = writeln!(out, "{}+0x{:x}", module.name.as_str(), address - module.bias);
            if known < bases.len() && !bases[..known].contains(&module.bias) {
                bases[known] = module.bias;
                known += 1;
            }
        }
        None => {
            let _ = writeln!(out, "?");
        }
    };
    if let Some(fault) = fault {
        let _ = write!(out, "Fault: ");
        location(out, fault);
    }
    let _ = writeln!(out, "Native stack (modules):");
    for (index, &address) in frames.iter().enumerate() {
        let _ = write!(out, "{index:02}: ");
        location(out, address);
    }
    let _ = writeln!(out, "Modules:");
    for &bias in &bases[..known] {
        let Some(module) = find_by_bias(bias) else { continue };
        let _ = write!(out, "{}", module.name.as_str());
        if let Some(build_id) = module.build_id() {
            write_identity(out, build_id);
        }
        let _ = writeln!(out);
    }
}

/// ` id=<debug id> code=<build ID>`: the debug id symbol files are filed
/// under (the build ID's first 16 bytes read as a GUID, then age 0, as
/// Breakpad's dump_syms names an ELF file), and the build ID itself.
fn write_identity(out: &mut (impl Write + ?Sized), build_id: &[u8]) {
    let mut guid = [0u8; 16];
    let used = build_id.len().min(16);
    guid[..used].copy_from_slice(&build_id[..used]);
    let data1 = u32::from_le_bytes([guid[0], guid[1], guid[2], guid[3]]);
    let data2 = u16::from_le_bytes([guid[4], guid[5]]);
    let data3 = u16::from_le_bytes([guid[6], guid[7]]);
    let _ = write!(out, " id={data1:08X}{data2:04X}{data3:04X}");
    for byte in &guid[8..] {
        let _ = write!(out, "{byte:02X}");
    }
    let _ = write!(out, "0 code=");
    for byte in build_id.iter().take(32) {
        let _ = write!(out, "{byte:02x}");
    }
}

/// A module file name of the shape the report accepts (`summary.rs`).
#[derive(Clone, Copy)]
struct Name {
    bytes: [u8; 64],
    len: usize,
}

impl Name {
    /// Only the file name, with anything but letters, digits, `_`, `-` and
    /// `.` replaced: a path names the user's folders.
    fn new(path: &[u8]) -> Name {
        let file = &path[path.iter().rposition(|&b| b == b'/').map_or(0, |i| i + 1)..];
        let mut name = Name { bytes: [0; 64], len: 0 };
        for &byte in file.iter().take(name.bytes.len()) {
            let allowed = byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.');
            name.bytes[name.len] = if allowed { byte } else { b'_' };
            name.len += 1;
        }
        name
    }

    fn as_str(&self) -> &str {
        match std::str::from_utf8(&self.bytes[..self.len]) {
            Ok(name) if !name.is_empty() => name,
            _ => "module",
        }
    }
}

struct Module {
    /// The load bias: an address minus it is the ELF virtual address.
    bias: usize,
    name: Name,
    id: [u8; 32],
    id_len: usize,
}

impl Module {
    fn build_id(&self) -> Option<&[u8]> {
        (self.id_len > 0).then(|| &self.id[..self.id_len])
    }
}

/// The loaded module whose segments hold `address`, from the loader's list.
fn find(address: usize) -> Option<Module> {
    search(Target::Address(address))
}

/// The loaded module with this load bias.
fn find_by_bias(bias: usize) -> Option<Module> {
    search(Target::Bias(bias))
}

#[derive(Clone, Copy)]
enum Target {
    Address(usize),
    Bias(usize),
}

fn search(target: Target) -> Option<Module> {
    struct Search {
        target: Target,
        found: Option<Module>,
    }
    unsafe extern "C" fn each(info: *mut libc::dl_phdr_info, _size: usize, search: *mut c_void) -> c_int {
        // SAFETY: the loader passes a live record per module, and `search`
        // is the Search below; program headers and notes lie in the
        // module's mapped segments.
        unsafe {
            let search = &mut *search.cast::<Search>();
            let info = &*info;
            let headers = std::slice::from_raw_parts(info.dlpi_phdr, usize::from(info.dlpi_phnum));
            let bias = info.dlpi_addr as usize;
            let inside = match search.target {
                Target::Bias(wanted) => wanted == bias,
                Target::Address(address) => headers.iter().any(|h| {
                    let start = bias.wrapping_add(h.p_vaddr as usize);
                    h.p_type == libc::PT_LOAD && address >= start && address - start < h.p_memsz as usize
                }),
            };
            if !inside {
                return 0;
            }
            let name = if info.dlpi_name.is_null() {
                &[][..]
            } else {
                CStr::from_ptr(info.dlpi_name).to_bytes()
            };
            let mut module = Module {
                bias,
                // The executable is listed with an empty name.
                name: match (name.is_empty(), EXE.get()) {
                    (true, Some(exe)) => *exe,
                    _ => Name::new(name),
                },
                id: [0; 32],
                id_len: 0,
            };
            for header in headers.iter().filter(|h| h.p_type == libc::PT_NOTE) {
                let notes = std::slice::from_raw_parts(bias.wrapping_add(header.p_vaddr as usize) as *const u8, header.p_memsz as usize);
                if let Some(id) = build_id(notes) {
                    module.id_len = id.len().min(module.id.len());
                    module.id[..module.id_len].copy_from_slice(&id[..module.id_len]);
                    break;
                }
            }
            search.found = Some(module);
            1
        }
    }
    let mut search = Search { target, found: None };
    // SAFETY: the callback only reads the records the loader gives it and
    // writes into `search`.
    unsafe { libc::dl_iterate_phdr(Some(each), (&raw mut search).cast()) };
    search.found
}

/// The GNU build ID in a PT_NOTE segment: notes of `namesz descsz type`,
/// then the name and the descriptor, each padded to 4 bytes.
fn build_id(mut notes: &[u8]) -> Option<&[u8]> {
    const NT_GNU_BUILD_ID: u32 = 3;
    let pad = |n: usize| n.checked_add(3).map(|n| n & !3);
    while notes.len() >= 12 {
        let word = |at: usize| u32::from_ne_bytes([notes[at], notes[at + 1], notes[at + 2], notes[at + 3]]) as usize;
        let (name_size, desc_size, kind) = (word(0), word(4), word(8) as u32);
        let name_end = 12usize.checked_add(pad(name_size)?)?;
        let desc_end = name_end.checked_add(pad(desc_size)?)?;
        if desc_end > notes.len() {
            return None;
        }
        if kind == NT_GNU_BUILD_ID && name_size == 4 && &notes[12..16] == b"GNU\0" && desc_size > 0 {
            return Some(&notes[name_end..name_end + desc_size]);
        }
        notes = &notes[desc_end..];
    }
    None
}

/// Another thread's stack, for a frozen window or a GPU stall: the thread
/// is sent [`snapshot::signal`] (tgkill), whose handler unwinds its own
/// stack into the fixed array below and says so; the asking thread waits
/// for it, then writes the modules itself. A thread held in the kernel
/// (a driver's call that does not return) does not run the handler: no
/// stack then. The Legacy Viewer has no equivalent: its watchdog ends a
/// frozen viewer with a crash dump of the main thread (LLWatchdogTimeout,
/// indra/newview/llwatchdog.cpp).
mod snapshot {
    use super::{FRAMES, capture};
    use std::ffi::{c_int, c_void};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    /// The request: its number in the high half, the thread asked in the
    /// low half; 0 when none. [`CLAIMED`] is set by the handler that took
    /// it, before it writes [`FRAMES_OUT`]: an answer that comes after its
    /// request gave up finds another value and writes nothing.
    pub(super) static ASKED: AtomicU64 = AtomicU64::new(0);
    pub(super) const CLAIMED: u64 = 1 << 63;
    pub(super) static FRAMES_OUT: [AtomicUsize; FRAMES] = [const { AtomicUsize::new(0) }; FRAMES];
    /// The frame count plus one once written, 0 before.
    pub(super) static WRITTEN: AtomicUsize = AtomicUsize::new(0);
    /// The handler is ours (`install_snapshot`).
    pub(super) static OWNED: AtomicBool = AtomicBool::new(false);
    /// One request at a time (the watchdog and the GPU stall).
    static ONE: Mutex<u64> = Mutex::new(0);
    /// How long a thread has to answer.
    const ANSWER: Duration = Duration::from_millis(500);

    /// A real-time signal nobody else in the process uses: the
    /// standard library and the C library leave the high ones alone.
    pub fn signal() -> c_int {
        libc::SIGRTMAX() - 2
    }

    /// [`on_snapshot`] as `sigaction` holds it.
    pub(super) fn handler() -> libc::sighandler_t {
        on_snapshot as extern "C" fn(c_int, *mut libc::siginfo_t, *mut c_void) as libc::sighandler_t
    }

    /// The thread goes on once the handler returns: its `errno` is kept.
    extern "C" fn on_snapshot(_signal: c_int, _info: *mut libc::siginfo_t, context: *mut c_void) {
        // SAFETY: the calling thread's errno, which a signal handler may
        // read and write (async-signal-safe).
        let errno = unsafe { *libc::__errno_location() };
        answer(context);
        // SAFETY: as above.
        unsafe { *libc::__errno_location() = errno };
    }

    fn answer(context: *mut c_void) {
        let asked = ASKED.load(Ordering::Acquire);
        // SAFETY: gettid has no argument and is async-signal-safe.
        if asked == 0 || asked & CLAIMED != 0 || asked as u32 as i32 != unsafe { libc::gettid() } {
            return;
        }
        let mut frames = [0usize; FRAMES];
        let count = capture(&mut frames);
        if ASKED
            .compare_exchange(asked, asked | CLAIMED, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        // SAFETY: the kernel's ucontext for this SA_SIGINFO handler.
        let (_, _, pc) = unsafe { super::fault(std::ptr::null(), context) };
        // The handler's frames and the signal trampoline come first: the
        // stack starts at the instruction the thread was interrupted on.
        let start = pc.and_then(|pc| frames[..count].iter().position(|&f| f == pc)).unwrap_or(0);
        for (slot, &frame) in FRAMES_OUT.iter().zip(&frames[start..count]) {
            slot.store(frame, Ordering::Relaxed);
        }
        WRITTEN.store(count - start + 1, Ordering::Release);
    }

    /// The thread's frames, if it answered in time.
    pub fn take(tid: i32) -> Option<Vec<usize>> {
        if !OWNED.load(Ordering::Acquire) || tid <= 0 {
            return None;
        }
        let mut number = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        // A library loaded since the start (a driver) may have taken the
        // signal: then sending it would run their handler, not ours.
        // SAFETY: reads the action into a zeroed struct we own.
        let ours = unsafe {
            let mut current: libc::sigaction = std::mem::zeroed();
            libc::sigaction(signal(), std::ptr::null(), &mut current) == 0 && current.sa_sigaction == handler()
        };
        if !ours {
            return None;
        }
        *number = (*number + 1) & 0x7FFF_FFFF;
        let asked = (*number << 32) | u64::from(tid as u32);
        WRITTEN.store(0, Ordering::Relaxed);
        ASKED.store(asked, Ordering::Release);
        // SAFETY: a thread of this process, sent a signal whose handler
        // was installed at start.
        let sent = unsafe { libc::syscall(libc::SYS_tgkill, libc::getpid(), tid, signal()) } == 0;
        if !sent {
            ASKED.store(0, Ordering::Release);
            return None;
        }
        let deadline = Instant::now() + ANSWER;
        let mut count;
        loop {
            count = WRITTEN.load(Ordering::Acquire);
            if count != 0 {
                break;
            }
            if Instant::now() >= deadline {
                // Withdrawn unless its handler has just claimed it, in
                // which case the frames are a few stores away.
                if ASKED.compare_exchange(asked, 0, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                    return None;
                }
                while count == 0 {
                    std::hint::spin_loop();
                    count = WRITTEN.load(Ordering::Acquire);
                }
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        ASKED.store(0, Ordering::Release);
        let frames: Vec<usize> = FRAMES_OUT[..count.saturating_sub(1)]
            .iter()
            .map(|f| f.load(Ordering::Relaxed))
            .collect();
        (!frames.is_empty()).then_some(frames)
    }
}

/// Installs the handler that answers [`Thread::write_stack`]; left alone
/// when the signal already has one.
fn install_snapshot() -> anyhow::Result<()> {
    let signal = snapshot::signal();
    // SAFETY: plain sigaction calls on zeroed structs we own; the handler
    // only touches static atomics and async-signal-safe calls.
    unsafe {
        let mut old: libc::sigaction = std::mem::zeroed();
        anyhow::ensure!(
            libc::sigaction(signal, std::ptr::null(), &mut old) == 0,
            "cannot read signal {signal}"
        );
        // Ours already (a second install), or another's: then no stacks.
        if old.sa_sigaction != libc::SIG_DFL {
            return Ok(());
        }
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = snapshot::handler();
        action.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK | libc::SA_RESTART;
        libc::sigemptyset(&mut action.sa_mask);
        anyhow::ensure!(
            libc::sigaction(signal, &action, std::ptr::null_mut()) == 0,
            "cannot handle signal {signal}"
        );
    }
    snapshot::OWNED.store(true, std::sync::atomic::Ordering::Release);
    Ok(())
}

/// A thread of this process, by its kernel id.
pub struct Thread(i32);

impl Thread {
    pub fn current() -> Option<Thread> {
        // SAFETY: no argument.
        Some(Thread(unsafe { libc::gettid() }))
    }

    pub fn open(id: u32) -> Option<Thread> {
        i32::try_from(id).ok().map(Thread)
    }

    /// Its stack as module + offset; false when it did not answer.
    pub fn write_stack(&self, out: &mut impl Write) -> bool {
        match snapshot::take(self.0) {
            Some(frames) => {
                write_modules(out, &frames, None);
                true
            }
            None => false,
        }
    }
}

/// This process's threads with their names (`aurora-render`…), from
/// `/proc/self/task/<id>/comm` (15 bytes at most).
pub fn named_threads() -> Vec<(u32, String)> {
    let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
        return Vec::new();
    };
    tasks
        .flatten()
        .filter_map(|task| {
            let id = task.file_name().to_str()?.parse().ok()?;
            let name = std::fs::read_to_string(task.path().join("comm")).ok()?;
            let name = name.trim_end_matches('\n');
            (!name.is_empty()).then(|| (id, name.to_owned()))
        })
        .collect()
}

/// A fault no Rust code can cause: a read of a page mapped without access.
pub fn demo_exception() -> ! {
    // SAFETY: an anonymous mapping of one page with no access; the read of
    // it raises SIGSEGV, which ends this offline test process.
    unsafe {
        let page = libc::mmap(
            std::ptr::null_mut(),
            4096,
            libc::PROT_NONE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        );
        if page != libc::MAP_FAILED {
            std::ptr::read_volatile(page.cast::<u8>());
        }
    }
    std::process::abort()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(name: &[u8], kind: u32, desc: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(name.len() as u32).to_ne_bytes());
        out.extend_from_slice(&(desc.len() as u32).to_ne_bytes());
        out.extend_from_slice(&kind.to_ne_bytes());
        out.extend_from_slice(name);
        out.resize(out.len().next_multiple_of(4), 0);
        out.extend_from_slice(desc);
        out.resize(out.len().next_multiple_of(4), 0);
        out
    }

    #[test]
    fn the_build_id_is_found_among_other_notes_and_bad_notes_end_the_search() {
        let id: Vec<u8> = (1..=20).collect();
        let mut notes = note(b"GNU\0", 1, &[0; 16]);
        notes.extend(note(b"stapsdt\0", 3, &[7; 5]));
        notes.extend(note(b"GNU\0", 3, &id));
        assert_eq!(build_id(&notes), Some(&id[..]));
        assert_eq!(build_id(&note(b"GNU\0", 1, &id)), None);
        // A size beyond the segment, then one that would wrap.
        let mut short = note(b"GNU\0", 3, &id);
        short.truncate(20);
        assert_eq!(build_id(&short), None);
        let mut huge = note(b"GNU\0", 3, &id);
        huge[4..8].copy_from_slice(&u32::MAX.to_ne_bytes());
        assert_eq!(build_id(&huge), None);
        assert_eq!(build_id(&[]), None);
    }

    #[test]
    fn the_debug_id_is_the_build_id_read_as_a_guid_as_breakpad_files_it() {
        let id: Vec<u8> = (1..=20).collect();
        let mut out = Vec::new();
        write_identity(&mut out, &id);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            " id=0403020106050807090A0B0C0D0E0F100 code=0102030405060708090a0b0c0d0e0f1011121314"
        );
        // A short build ID is padded to the GUID's 16 bytes.
        let mut out = Vec::new();
        write_identity(&mut out, &[0xAB; 8]);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            " id=ABABABABABABABAB00000000000000000 code=abababababababab"
        );
    }

    #[test]
    fn module_names_keep_only_the_file_name_in_the_report_s_alphabet() {
        assert_eq!(Name::new(b"/usr/lib/libvulkan_radeon.so").as_str(), "libvulkan_radeon.so");
        assert_eq!(Name::new(b"/home/Private Name/my viewer").as_str(), "my_viewer");
        assert_eq!(Name::new(b"").as_str(), "module");
        assert_eq!(Name::new(&[b'a'; 100]).as_str().len(), 64);
    }

    #[test]
    fn stacks_are_written_as_module_offsets_with_build_ids() {
        let _ = EXE.set(Name::new(b"aurora_viewer-test"));
        let mut out = Vec::new();
        write_stack(&mut out);
        let text = String::from_utf8(out).unwrap();
        let frames: Vec<_> = text.lines().filter(|line| line.contains("+0x")).collect();
        assert!(frames.len() > 3, "{text}");
        for frame in &frames {
            let offset = frame.rsplit_once("+0x").unwrap().1;
            assert!(u64::from_str_radix(offset, 16).unwrap() < 1 << 32, "{frame}");
        }
        let modules: Vec<_> = text.split_once("Modules:\n").unwrap().1.lines().collect();
        // The test binary itself (listed by the loader without a name) and
        // the C library, each with its build ID.
        let own = modules
            .iter()
            .find(|line| line.starts_with("aurora_viewer-test "))
            .expect("own module");
        assert!(own.contains(" id=") && own.contains(" code="), "{own}");
        assert!(modules.iter().any(|line| line.starts_with("libc.so")), "{text}");
    }

    #[test]
    fn the_fault_location_and_unknown_frames_are_written() {
        let here = stacks_are_written_as_module_offsets_with_build_ids as *const () as usize;
        let mut out = Vec::new();
        write_modules(&mut out, &[here, 8], Some(here));
        let text = String::from_utf8(out).unwrap();
        let mut lines = text.lines();
        let fault = lines.next().unwrap();
        assert!(fault.starts_with("Fault: ") && fault.contains("+0x"), "{text}");
        assert_eq!(lines.next(), Some("Native stack (modules):"));
        assert_eq!(lines.next(), Some(&*format!("00: {}", &fault["Fault: ".len()..])));
        assert_eq!(lines.next(), Some("01: ?"));
        assert_eq!(text.split_once("Modules:\n").unwrap().1.lines().count(), 1, "{text}");
    }

    #[test]
    fn a_blocked_thread_s_stack_is_read_from_another_thread() {
        install_snapshot().unwrap();
        let _ = EXE.set(Name::new(b"aurora_viewer-test"));
        let (ids, id) = std::sync::mpsc::channel();
        let (release, wait) = std::sync::mpsc::channel::<()>();
        let blocked = std::thread::Builder::new()
            .name("aurora-test-blk".into())
            .spawn(move || {
                // SAFETY: no argument.
                ids.send(unsafe { libc::gettid() }).unwrap();
                let _ = wait.recv();
            })
            .unwrap();
        let id = id.recv().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        let mut out = Vec::new();
        assert!(Thread(id).write_stack(&mut out));
        assert!(named_threads().contains(&(id as u32, "aurora-test-blk".into())));
        // The thread goes on after it: its wait was only interrupted.
        release.send(()).unwrap();
        blocked.join().unwrap();
        let text = String::from_utf8(out).unwrap();
        let frames: Vec<_> = text.lines().filter(|line| line.contains("+0x")).collect();
        // From the wait in the C library down to the thread's start, through
        // this test binary's own code, without the handler's frames.
        assert!(frames.len() > 4, "{text}");
        assert!(frames[0].contains("libc.so"), "{text}");
        assert!(frames.iter().any(|f| f.contains("aurora_viewer-test")), "{text}");
        // A thread that does not exist answers nothing.
        assert!(!Thread(i32::MAX).write_stack(&mut Vec::new()));
    }

    #[test]
    fn signal_names_are_the_fatal_ones() {
        for signal in SIGNALS {
            assert!(signal_name(signal).is_some());
        }
        assert_eq!(signal_name(libc::SIGTERM), None);
    }
}
