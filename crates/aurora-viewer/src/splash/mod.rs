//! The startup splash: a small window that shows at once on a launch,
//! before the renderer, which can take many seconds to compile its
//! pipelines when no cache holds them, with nothing on screen until then
//! (users then launched the viewer again and again). The Legacy Viewer shows
//! a splash bitmap from `LLSplashScreen::show` (llappviewer.cpp,
//! llwindowwin32.cpp) during the same steps; this one says what the start
//! is doing and how far it is.
//!
//! It is another process of the same executable (`--splash`, like the crash
//! window), drawn on the CPU (`window.rs`): the viewer's main thread is held
//! by the renderer's start, and the splash must not need the GPU it waits
//! for. A thread of the viewer sends it the start's step
//! (`progress::step_of`: the viewer's phase and
//! `aurora_render::start_progress`) on its standard input; it closes when
//! the viewer's window shows its first frame, or when the viewer is gone.
//!
//! An update shows the same window ([`UpdateWindow`]) from the updater's
//! helper, from the old viewer's close until the new viewer's own splash is
//! on screen: the switch (old process ending, folders swapped, a cache set
//! aside, the antivirus reading the new executable) took several seconds
//! with nothing of Aurora on screen. The new viewer's splash, once shown,
//! tells the helper (`aurora_updater::install::acknowledge_splash`), which
//! closes its window then.
//!
//! Not on macOS: the main window is visible from its creation there (Metal
//! presents into a visible layer), and a second process would add a second
//! icon to the Dock. `AURORA_SPLASH=0` turns it off; automated captures
//! (`AURORA_CAPTURE`) have none unless `AURORA_SPLASH_CAPTURE` asks for it.

pub mod progress;
mod window;

use progress::{Message, Step, UpdateStep, ViewerPhase};
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// The argument of the splash process.
const ARG: &str = "--splash";

static VIEWER: AtomicU8 = AtomicU8::new(ViewerPhase::Starting as u8);
/// The splash is not wanted any more (the start failed).
static CLOSE: AtomicBool = AtomicBool::new(false);
/// Milliseconds from the viewer's start to the splash on screen (0: not yet).
static SHOWN_MS: AtomicU64 = AtomicU64::new(0);
static STARTED: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// The splash process itself: shows the window and returns true.
pub fn helper() -> anyhow::Result<bool> {
    if !std::env::args_os().any(|a| a == ARG) {
        return Ok(false);
    }
    // the language of the viewer: its saved choice, unless --language
    let language = crate::settings::Settings::language_at_start();
    aurora_i18n::select(&language, crate::cli::get().language.as_deref());
    window::run()?;
    Ok(true)
}

/// Whether this launch shows a splash, from its environment.
fn wanted(splash: Option<&str>, capture: bool, splash_capture: bool) -> bool {
    if cfg!(target_os = "macos") {
        return false;
    }
    match splash.map(str::trim) {
        Some("0") => false,
        Some("1") => true,
        _ => !capture || splash_capture,
    }
}

fn wanted_here() -> bool {
    wanted(
        std::env::var("AURORA_SPLASH").ok().as_deref(),
        std::env::var_os("AURORA_CAPTURE").is_some(),
        std::env::var_os("AURORA_SPLASH_CAPTURE").is_some(),
    )
}

/// The splash process of this executable, its lines on its standard input;
/// `shown`: its line back once on screen is read.
fn spawn(title: Option<&str>, language: Option<&str>, shown: bool) -> std::io::Result<std::process::Child> {
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command.arg(ARG);
    if let Some(title) = title {
        command.arg("--title").arg(title);
    }
    if let Some(language) = language {
        command.arg("--language").arg(language);
    }
    command
        .stdin(std::process::Stdio::piped())
        .stdout(if shown {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // the development build is a console program: no console window
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    command.spawn()
}

/// The updater helper's window over the switch to a new version
/// (`aurora_updater::install::run_helper`, before anything else of `main`).
/// Started at the first stage; gone when handed over or dropped.
#[derive(Default)]
pub struct UpdateWindow {
    child: Option<std::process::Child>,
    started: bool,
}

impl UpdateWindow {
    pub fn stage(&mut self, version: &str, stage: aurora_updater::install::HelperStage) {
        let Some(step) = UpdateStep::of(stage) else {
            self.close();
            return;
        };
        if !self.started {
            self.started = true;
            if !wanted_here() {
                return;
            }
            // no log yet in the helper: a missing window only leaves the
            // switch silent, as it was
            self.child = spawn(None, None, false).ok();
            self.send(&Message::Version(version.to_owned()));
        }
        self.send(&Message::Step(Step::Update(step)));
    }

    fn send(&mut self, message: &Message) {
        let Some(child) = &mut self.child else { return };
        let sent = child.stdin.as_mut().is_some_and(|input| {
            input
                .write_all(format!("{}\n", message.line()).as_bytes())
                .and_then(|()| input.flush())
                .is_ok()
        });
        if !sent {
            self.close();
        }
    }

    /// Its input closed: the window ends on its own (never waited for).
    fn close(&mut self) {
        if let Some(mut child) = self.child.take() {
            if let Some(mut input) = child.stdin.take() {
                let _ = input.write_all(format!("{}\n", Message::Done.line()).as_bytes());
            }
            // reaped by the system once the helper exits
            drop(child);
        }
    }
}

impl Drop for UpdateWindow {
    fn drop(&mut self) {
        self.close();
    }
}

/// Starts the splash, when wanted; never waits for it.
pub fn start() {
    let started = *STARTED.get_or_init(Instant::now);
    if !wanted_here() {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("aurora-splash".into())
        .spawn(move || feed(started));
    if let Err(e) = spawned {
        eprintln!("startup splash: no thread ({e})");
    }
}

/// The viewer reads its settings and caches (after the log is open).
pub fn settings() {
    VIEWER.fetch_max(ViewerPhase::Settings as u8, Ordering::Relaxed);
}

/// The viewer's window showed its first frame: the splash closes.
pub fn shown() {
    VIEWER.store(ViewerPhase::Shown as u8, Ordering::Relaxed);
    let started = STARTED.get().copied();
    match (SHOWN_MS.load(Ordering::Relaxed), started) {
        (0, Some(t)) => log::info!(
            "startup splash: not on screen before the window, which showed {} ms after the start",
            t.elapsed().as_millis()
        ),
        (ms, Some(t)) => log::info!(
            "startup splash: on screen {ms} ms after the start, closed after {} ms",
            t.elapsed().as_millis()
        ),
        _ => {}
    }
}

/// The start failed (no renderer, no window): the splash closes before the
/// message that says so.
pub fn close() {
    CLOSE.store(true, Ordering::Relaxed);
}

/// The splash process, then its steps until the viewer shows.
fn feed(started: Instant) {
    let cli = crate::cli::get();
    let mut child = match spawn(cli.title.as_deref(), cli.language.as_deref(), true) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("startup splash: not started ({e})");
            return;
        }
    };
    // its one line back: on screen
    if let Some(out) = child.stdout.take() {
        let _ = std::thread::Builder::new().name("aurora-splash-shown".into()).spawn(move || {
            use std::io::BufRead;
            for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                if line.trim() == "shown" {
                    SHOWN_MS.store((started.elapsed().as_millis() as u64).max(1), Ordering::Relaxed);
                    // started by an update: its helper's window can go now
                    aurora_updater::install::acknowledge_splash();
                }
            }
        });
    }
    let Some(mut input) = child.stdin.take() else {
        return;
    };
    let mut sent: Option<Step> = None;
    loop {
        if CLOSE.load(Ordering::Relaxed) {
            break;
        }
        let viewer = ViewerPhase::from_u8(VIEWER.load(Ordering::Relaxed));
        let step = progress::step_of(viewer, aurora_render::start_progress::snapshot());
        let mut lines = String::new();
        if sent != Some(step) {
            lines.push_str(&Message::Step(step).line());
            lines.push('\n');
            sent = Some(step);
        }
        if viewer == ViewerPhase::Shown {
            lines.push_str(&Message::Done.line());
            lines.push('\n');
        }
        if !lines.is_empty() && (input.write_all(lines.as_bytes()).is_err() || input.flush().is_err()) {
            break;
        }
        if viewer == ViewerPhase::Shown {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    // its input closed: the splash ends (it holds on with
    // AURORA_DEMO_SPLASH_HOLD)
    drop(input);
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_have_no_splash_unless_they_capture_it() {
        let shows = !cfg!(target_os = "macos");
        assert_eq!(wanted(None, false, false), shows);
        assert!(!wanted(Some("0"), false, false));
        assert!(!wanted(None, true, false));
        assert_eq!(wanted(None, true, true), shows);
        assert_eq!(wanted(Some("1"), true, false), shows);
        assert!(!wanted(Some("0"), true, true));
    }
}
