//! The session's sign of life, for the report of a session that ends
//! without a word of its own (killed, frozen then ended, Windows gone):
//! every few seconds a thread writes `alive.txt` with the time, the last
//! time the window answered and the viewer's last state (screen, frame
//! rate, memory, graphics; numbers and fixed names only, see `State`).
//! When the window stops answering (Windows' own test, the one behind
//! "Not responding"), it saves the stacks of the main and render threads
//! in `freeze.txt`, removed again if the window answers once more.
//! Legacy Viewer reference: llwatchdog.cpp (LLWatchdogTimeout), which ends
//! a frozen viewer itself; this one only observes, the user decides.

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
#[cfg(windows)]
use std::sync::atomic::Ordering;
#[cfg(windows)]
use std::time::Instant;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How often the window is tested.
const TICK: Duration = Duration::from_secs(1);
/// How often `alive.txt` is written.
const ALIVE_EVERY: Duration = Duration::from_secs(5);
/// While the window stays frozen, the stacks are read again this often: the
/// last ones show where it stays.
const RECAPTURE: Duration = Duration::from_secs(15);
/// `IsHungAppWindow` answers after this long without a message taken.
pub const HUNG_AFTER: u64 = 5;
/// The threads whose stacks a freeze report shows, after the main one.
const THREADS: &[&str] = &["aurora-render"];

static WINDOW: AtomicUsize = AtomicUsize::new(0);
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
    #[cfg(not(windows))]
    let _ = window;
}

/// Started from the main thread, whose handle it keeps.
pub fn start(dir: &Path) {
    #[cfg(windows)]
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
    #[cfg(not(windows))]
    let _ = dir;
}

fn unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

#[cfg(windows)]
fn run(dir: &Path, main: Option<&super::native::Thread>) {
    let mut hung_since: Option<Instant> = None;
    let mut captured: Option<Instant> = None;
    let mut responsive: Option<u64> = None;
    let mut written: Option<Instant> = None;
    loop {
        std::thread::sleep(TICK);
        let window = WINDOW.load(Ordering::Relaxed);
        if window != 0 {
            if super::native::window_hung(window) {
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

#[cfg(windows)]
fn save_freeze(dir: &Path, main: Option<&super::native::Thread>, seconds: u64) {
    save_stacks(dir, "freeze", &format!("Freeze: no answer for {seconds} s"), main);
}

/// The render thread kept a frame in the GPU (the window still answers,
/// `aurora_render::Renderer::stalled_for`): its stack in `gpu-stall.txt`.
#[cfg(windows)]
pub fn save_gpu_stall(dir: &Path, seconds: u64) {
    save_stacks(dir, "gpu-stall", &format!("GPU stall: no frame for {seconds} s"), None);
}

/// `<name>.txt`: a header line, then the main thread's stack when given and
/// those of [`THREADS`].
#[cfg(windows)]
fn save_stacks(dir: &Path, name: &str, header: &str, main: Option<&super::native::Thread>) {
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
