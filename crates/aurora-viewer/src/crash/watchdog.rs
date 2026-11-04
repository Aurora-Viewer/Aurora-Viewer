//! The session's sign of life, for the report of a session that ends
//! without a word of its own (killed, frozen then ended, Windows gone):
//! every few seconds a thread writes `alive.txt` with the time, the last
//! time the window answered and the viewer's last state (screen, frame
//! rate, memory, graphics; numbers and fixed names only, see `State`).
//! When the window stops answering (Windows' own test, the one behind
//! "Not responding"; on Linux the main loop not going round for as long),
//! it saves the stacks of the main and render threads in `freeze.txt`,
//! removed again if the window answers once more.
//! Legacy Viewer reference: llwatchdog.cpp (LLWatchdogTimeout), which ends
//! a frozen viewer itself; this one only observes, the user decides.

use std::path::Path;
use std::sync::Mutex;
#[cfg(target_os = "linux")]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::AtomicUsize;
#[cfg(any(windows, target_os = "linux"))]
use std::sync::atomic::Ordering;
#[cfg(any(windows, target_os = "linux"))]
use std::time::Instant;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How often the window is tested.
const TICK: Duration = Duration::from_secs(1);
/// How often `alive.txt` is written.
const ALIVE_EVERY: Duration = Duration::from_secs(5);
/// While the window stays frozen, the stacks are read again this often: the
/// last ones show where it stays.
const RECAPTURE: Duration = Duration::from_secs(15);
/// `IsHungAppWindow` answers after this long without a message taken; on
/// Linux the main loop is taken for frozen after as long without a pass.
pub const HUNG_AFTER: u64 = 5;
/// The threads whose stacks a freeze report shows, after the main one.
const THREADS: &[&str] = &["aurora-render"];

static WINDOW: AtomicUsize = AtomicUsize::new(0);
/// Linux: the main loop's last pass, in milliseconds since [`EPOCH`].
#[cfg(target_os = "linux")]
static PUMPED: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "linux")]
static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
static STATE: Mutex<Option<State>> = Mutex::new(None);

/// What the viewer was doing, noted by the app every few seconds. Only
/// numbers and names fixed in the code: the report publishes it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    /// `login`, `loading` or `world`.
    pub screen: &'static str,
    pub fps: f32,
    pub frame_ms: f32,
    pub window: (u32, u32),
    /// The graphics preset the settings match, `custom` otherwise.
    pub preset: &'static str,
    pub vram_used_mb: Option<u64>,
    pub vram_budget_mb: Option<u64>,
    /// Texture memory pressure step and its maximum.
    pub texture_pressure: (u32, u32),
    pub objects: usize,
    pub avatars: usize,
    pub regions: usize,
    /// Swapchain recreations (window resizes among them) in the 10 s
    /// before this note, or before the GPU was lost.
    pub swapchain_10s: u64,
}

impl State {
    fn lines(&self, out: &mut String) {
        use std::fmt::Write;
        let _ = writeln!(out, "screen={}", self.screen);
        let _ = writeln!(out, "fps={:.1}", self.fps);
        let _ = writeln!(out, "frame_ms={:.1}", self.frame_ms);
        let _ = writeln!(out, "window={}x{}", self.window.0, self.window.1);
        let _ = writeln!(out, "preset={}", self.preset);
        if let (Some(used), Some(budget)) = (self.vram_used_mb, self.vram_budget_mb) {
            let _ = writeln!(out, "vram_mb={used}/{budget}");
        }
        let _ = writeln!(out, "texture_pressure={}/{}", self.texture_pressure.0, self.texture_pressure.1);
        let _ = writeln!(out, "objects={}", self.objects);
        let _ = writeln!(out, "avatars={}", self.avatars);
        let _ = writeln!(out, "regions={}", self.regions);
        let _ = writeln!(out, "swapchain_10s={}", self.swapchain_10s);
    }
}

/// The app's latest state, kept for the next `alive.txt`.
pub fn note(state: State) {
    if let Ok(mut kept) = STATE.try_lock() {
        *kept = Some(state);
    }
}

/// The window to test, once it exists.
pub fn watch_window(window: &winit::window::Window) {
    #[cfg(windows)]
    {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = window.window_handle()
            && let RawWindowHandle::Win32(handle) = handle.as_raw()
        {
            WINDOW.store(handle.hwnd.get() as usize, Ordering::Relaxed);
        }
    }
    #[cfg(target_os = "linux")]
    {
        let _ = window;
        pumped();
        WINDOW.store(1, Ordering::Relaxed);
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    let _ = window;
}

/// From the main loop at each pass (`about_to_wait`, which runs at every
/// pass of the loop as it polls): Linux has no system test of a hung
/// window, so the loop's own passes stand for the messages Windows sees
/// taken.
pub fn pumped() {
    #[cfg(target_os = "linux")]
    PUMPED.store(since_epoch(), Ordering::Relaxed);
}

#[cfg(target_os = "linux")]
fn since_epoch() -> u64 {
    EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Whether the window stopped answering.
#[cfg(windows)]
fn window_hung(window: usize) -> bool {
    super::native::window_hung(window)
}

#[cfg(target_os = "linux")]
fn window_hung(_window: usize) -> bool {
    since_epoch().saturating_sub(PUMPED.load(Ordering::Relaxed)) >= HUNG_AFTER * 1000
}

/// Started from the main thread, whose handle it keeps.
pub fn start(dir: &Path) {
    #[cfg(any(windows, target_os = "linux"))]
    {
        let main = super::native::Thread::current();
        let dir = dir.to_owned();
        if let Err(error) = std::thread::Builder::new()
            .name("aurora-watchdog".into())
            .spawn(move || run(&dir, main.as_ref()))
        {
            eprintln!("crash watchdog not started: {error}");
        }
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    let _ = dir;
}

fn unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

#[cfg(any(windows, target_os = "linux"))]
fn run(dir: &Path, main: Option<&super::native::Thread>) {
    let mut hung_since: Option<Instant> = None;
    let mut captured: Option<Instant> = None;
    let mut responsive: Option<u64> = None;
    let mut written: Option<Instant> = None;
    loop {
        std::thread::sleep(TICK);
        let window = WINDOW.load(Ordering::Relaxed);
        if window != 0 {
            if window_hung(window) {
                let since = *hung_since.get_or_insert_with(Instant::now);
                if captured.is_none_or(|at| at.elapsed() >= RECAPTURE) {
                    captured = Some(Instant::now());
                    save_freeze(dir, main, since.elapsed().as_secs() + HUNG_AFTER);
                }
            } else {
                responsive = Some(unix());
                if let Some(since) = hung_since.take() {
                    captured = None;
                    let _ = std::fs::remove_file(dir.join("freeze.txt"));
                    // Only now: the frozen thread may have held the logger.
                    log::warn!(
                        "the window did not answer for about {} s, then answered again",
                        since.elapsed().as_secs() + HUNG_AFTER
                    );
                }
            }
        }
        if written.is_none_or(|at| at.elapsed() >= ALIVE_EVERY) {
            written = Some(Instant::now());
            write_alive(dir, responsive);
        }
    }
}

/// Written apart then renamed: a session stopped midway leaves the last
/// complete one.
fn write_alive(dir: &Path, responsive: Option<u64>) {
    use std::fmt::Write;
    let mut text = format!("tick={}\n", unix());
    if let Some(at) = responsive {
        let _ = writeln!(text, "responsive={at}");
    }
    if let Some(memory) = crate::sys_probe::process_memory() {
        let _ = writeln!(text, "ram_mb={}", memory.working_set >> 20);
    }
    if let Some(state) = STATE.try_lock().ok().and_then(|s| s.clone()) {
        state.lines(&mut text);
    }
    let pending = dir.join("alive.pending");
    if std::fs::write(&pending, text).is_ok() {
        let _ = std::fs::rename(pending, dir.join("alive.txt"));
    }
}

#[cfg(any(windows, target_os = "linux"))]
fn save_freeze(dir: &Path, main: Option<&super::native::Thread>, seconds: u64) {
    save_stacks(dir, "freeze", &format!("Freeze: no answer for {seconds} s"), main);
}

/// The render thread kept a frame in the GPU (the window still answers,
/// `aurora_render::Renderer::stalled_for`): its stack in `gpu-stall.txt`.
#[cfg(windows)]
pub fn save_gpu_stall(dir: &Path, seconds: u64) {
    save_stacks(dir, "gpu-stall", &format!("GPU stall: no frame for {seconds} s"), None);
}

/// On Linux the render thread's stack is asked for by a signal, which a
/// thread held in the driver may take up to half a second to refuse: the
/// header is written at once (all the next start's safe graphics offer
/// needs), the stack by a thread of its own, the main thread going on.
/// Kept only while the stall is (`crash::gpu_answered` removes the file):
/// the stacks replace the header only if it is still there once read, so a
/// GPU that answered during the reading leaves no stall behind.
#[cfg(target_os = "linux")]
pub fn save_gpu_stall(dir: &Path, seconds: u64) {
    let header = format!("GPU stall: no frame for {seconds} s");
    let answers = GPU_ANSWERS.load(Ordering::SeqCst);
    let pending = dir.join("gpu-stall.pending");
    if std::fs::write(&pending, format!("{header}\n")).is_err() || std::fs::rename(&pending, dir.join("gpu-stall.txt")).is_err() {
        return;
    }
    let dir = dir.to_owned();
    let _ = std::thread::Builder::new().name("aurora-crash-stall".into()).spawn(move || {
        let out = stacks(&header, None);
        if GPU_ANSWERS.load(Ordering::SeqCst) == answers && dir.join("gpu-stall.txt").exists() {
            write_stacks(&dir, "gpu-stall", out);
            // an answer between the test and the rename may have removed
            // the header before the stacks took its place
            if GPU_ANSWERS.load(Ordering::SeqCst) != answers {
                let _ = std::fs::remove_file(dir.join("gpu-stall.txt"));
            }
        }
    });
}

/// How many times the GPU answered again after a stall
/// ([`gpu_answered`]): a stall's stacks, read by a thread of their own,
/// are dropped when it changed meanwhile.
#[cfg(target_os = "linux")]
static GPU_ANSWERS: AtomicU64 = AtomicU64::new(0);

/// Called by `crash::gpu_answered` before it removes `gpu-stall.txt`.
#[cfg(target_os = "linux")]
pub fn gpu_answered() {
    GPU_ANSWERS.fetch_add(1, Ordering::SeqCst);
}

/// Elsewhere no other thread's stack can be read: the header alone, which
/// is all the next start's safe graphics offer needs.
#[cfg(not(any(windows, target_os = "linux")))]
pub fn save_gpu_stall(dir: &Path, seconds: u64) {
    let pending = dir.join("gpu-stall.pending");
    if std::fs::write(&pending, format!("GPU stall: no frame for {seconds} s\n")).is_ok() {
        let _ = std::fs::rename(pending, dir.join("gpu-stall.txt"));
    }
}

/// `<name>.txt`: a header line, then the main thread's stack when given and
/// those of [`THREADS`].
#[cfg(any(windows, target_os = "linux"))]
fn save_stacks(dir: &Path, name: &str, header: &str, main: Option<&super::native::Thread>) {
    write_stacks(dir, name, stacks(header, main));
}

#[cfg(any(windows, target_os = "linux"))]
fn stacks(header: &str, main: Option<&super::native::Thread>) -> Vec<u8> {
    let mut out = format!("{header}\n").into_bytes();
    if let Some(main) = main {
        out.extend_from_slice(b"Thread: main\n");
        main.write_stack(&mut out);
    }
    for (id, name) in super::native::named_threads() {
        if THREADS.contains(&name.as_str())
            && let Some(thread) = super::native::Thread::open(id)
        {
            out.extend_from_slice(format!("Thread: {name}\n").as_bytes());
            thread.write_stack(&mut out);
        }
    }
    out
}

#[cfg(any(windows, target_os = "linux"))]
fn write_stacks(dir: &Path, name: &str, out: Vec<u8>) {
    let pending = dir.join(format!("{name}.pending"));
    if std::fs::write(&pending, out).is_ok() {
        let _ = std::fs::rename(pending, dir.join(format!("{name}.txt")));
    }
}

/// The values of `alive.txt`, by key.
pub fn read_alive(dir: &Path) -> Vec<(String, String)> {
    super::read_limited(&dir.join("alive.txt"), 4096)
        .map(|text| {
            text.lines()
                .filter_map(|line| line.split_once('='))
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect()
        })
        .unwrap_or_default()
}
