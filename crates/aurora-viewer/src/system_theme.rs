//! Follow the system theme (Preferences › Appearance › Skin): the light or
//! dark mode and the accent colour of the desktop become the interface's
//! colours (`crate::interface_colors`), as they change. Off by default.
//! Aurora's own; the Legacy Viewer has no such option.
//!
//! Where each value comes from:
//! - Linux: the XDG desktop portal's `org.freedesktop.appearance` settings
//!   (`color-scheme`, `accent-color`) and its `SettingChanged` signal, read
//!   on a thread of their own; winit does not report a theme on X11 or
//!   Wayland. A backend without `accent-color` (xdg-desktop-portal-gtk)
//!   leaves the skin's accent; without a portal the option does nothing.
//! - Windows and macOS: the mode from winit (`Window::theme`,
//!   `WindowEvent::ThemeChanged`). The accent is read again once a second
//!   (`UISettings`, `NSColor::controlAccentColor`), like the skin's files;
//!   a change notice is a later step (TASKS.md).
//!
//! A colour the user chose in Preferences wins over the system's (`colors`).
//! Dark keeps the skin's own background and text; a system with no
//! preference counts as dark, the viewer's own look.

use crate::interface_colors::InterfaceColors;
use crate::settings::Rgba;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once, OnceLock, PoisonError};
use std::time::Duration;

/// The system's mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Scheme {
    #[default]
    NoPreference,
    Dark,
    Light,
}

/// What the system asks for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Look {
    pub scheme: Scheme,
    pub accent: Option<[u8; 3]>,
}

/// Background and text of a light system: `interface_colors` derives the
/// bars, fields and buttons from them, and keeps everything readable.
const LIGHT_SURFACE: Rgba = [0xF2, 0xF3, 0xF7, 255];
const LIGHT_TEXT: Rgba = [0x1B, 0x1E, 0x26, 255];

/// How long the start waits for the first reading, so that a light system
/// does not open on a dark frame. A slow bus only delays the follow.
const FIRST_READ: Duration = Duration::from_millis(150);

static LOOK: Mutex<Look> = Mutex::new(Look {
    scheme: Scheme::NoPreference,
    accent: None,
});
static CHANGED: AtomicBool = AtomicBool::new(false);

/// The interface's colours under `look`: each colour the user chose stays,
/// the others follow the system. Background and text go as a pair, as one
/// alone from the system could not be read on the other the user chose.
pub fn colors(look: Look, chosen: &InterfaceColors) -> InterfaceColors {
    let accent = chosen.accent.or(look.accent.map(|[r, g, b]| [r, g, b, 255]));
    let (surface, text) = if look.scheme == Scheme::Light && chosen.surface.is_none() && chosen.text.is_none() {
        (Some(LIGHT_SURFACE), Some(LIGHT_TEXT))
    } else {
        (chosen.surface, chosen.text)
    };
    InterfaceColors { accent, surface, text }
}

/// The system's look as last read.
pub fn current() -> Look {
    *LOOK.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Whether the look changed since the last call.
pub fn take_changed() -> bool {
    CHANGED.swap(false, Ordering::Relaxed)
}

/// What the system gives, for Preferences to say so: a desktop may publish
/// the mode alone, or nothing (Hyprland has no accent of its own, and its
/// portal no settings, xdg-desktop-portal-gtk gives the mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Given {
    Everything,
    ModeOnly,
    Nothing,
}

/// Before the first read nothing is known, so nothing is reported missing;
/// Windows and macOS always give their mode.
fn given_by(read: bool, mode: bool, accent: bool) -> Given {
    match (read, mode, accent) {
        (false, ..) | (true, true, true) | (true, false, true) => Given::Everything,
        (true, true, false) => Given::ModeOnly,
        (true, false, false) => Given::Nothing,
    }
}

pub fn given() -> Given {
    given_by(
        READ.load(Ordering::Relaxed),
        MODE.load(Ordering::Relaxed),
        current().accent.is_some(),
    )
}

/// The first read is done, or a late portal brought the mode.
fn reported(mode: bool) {
    MODE.store(mode, Ordering::Relaxed);
    READ.store(true, Ordering::Relaxed);
}

fn update(change: impl FnOnce(&mut Look)) {
    let mut look = LOOK.lock().unwrap_or_else(PoisonError::into_inner);
    let before = *look;
    change(&mut look);
    if *look != before {
        CHANGED.store(true, Ordering::Relaxed);
    }
}

/// Starts following, once: nothing runs before the option is on. `wait`
/// gives the first read a moment, for a first frame in the system's colours;
/// the frame loop never waits, the look arrives as a change.
pub fn start(wait: bool) {
    static START: Once = Once::new();
    START.call_once(|| {
        let (ready, first) = std::sync::mpsc::channel();
        let looks = demo_looks();
        let spawned = std::thread::Builder::new().name("system-theme".into()).spawn(move || match looks {
            Some(demo) => run_demo(demo, ready),
            None => backend::run(ready),
        });
        match spawned {
            Ok(_) if wait => {
                let _ = first.recv_timeout(FIRST_READ);
            }
            Ok(_) => {}
            Err(e) => log::warn!("system theme: not followed ({e})"),
        }
    });
}

/// Whether the option is on now (every frame): the thread outlives it, and
/// a read that repeats stops while nothing follows it.
pub fn set_active(on: bool) {
    ACTIVE.store(on, Ordering::Relaxed);
}

#[cfg(any(windows, test))]
fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// Once a frame while following.
pub fn tick() {
    if !scripted() {
        backend::tick();
    }
}

/// The mode of the window, on the systems where winit reports it. Linux's
/// comes from the portal; a demo scenario keeps its own.
pub fn window_theme(theme: Option<winit::window::Theme>) {
    if cfg!(target_os = "linux") || scripted() {
        return;
    }
    let scheme = match theme {
        Some(winit::window::Theme::Dark) => Scheme::Dark,
        Some(winit::window::Theme::Light) => Scheme::Light,
        None => Scheme::NoPreference,
    };
    update(|look| look.scheme = scheme);
}

/// The portal's `color-scheme`; an unknown value counts as no preference,
/// as its documentation asks.
#[cfg(any(target_os = "linux", test))]
fn scheme_from_portal(value: u32) -> Scheme {
    match value {
        1 => Scheme::Dark,
        2 => Scheme::Light,
        _ => Scheme::NoPreference,
    }
}

/// An sRGB colour of 0–1 components (the portal's `accent-color`, macOS);
/// one out of range means no accent, as the portal documents.
#[cfg(any(target_os = "linux", target_os = "macos", test))]
fn accent_from_unit(r: f64, g: f64, b: f64) -> Option<[u8; 3]> {
    let byte = |c: f64| (0.0..=1.0).contains(&c).then(|| (c * 255.0).round() as u8);
    Some([byte(r)?, byte(g)?, byte(b)?])
}

/// `AURORA_DEMO_SYSTEM_THEME`, demo mode only; read once, as this is asked
/// every frame.
fn demo_active() -> bool {
    static ACTIVE: OnceLock<bool> = OnceLock::new();
    *ACTIVE.get_or_init(|| std::env::var_os("AURORA_DEMO").is_some() && std::env::var_os("AURORA_DEMO_SYSTEM_THEME").is_some())
}

static ACTIVE: AtomicBool = AtomicBool::new(false);
static READ: AtomicBool = AtomicBool::new(false);
static MODE: AtomicBool = AtomicBool::new(false);

/// Whether the interface follows the system: the option, or a demo run's
/// scenario, whatever the settings say.
pub fn following(settings: &crate::settings::Settings) -> bool {
    settings.follow_system_theme || demo_active()
}

/// The scenario's looks; `system` follows the real system in a demo run.
fn demo_looks() -> Option<DemoLooks> {
    demo_active()
        .then(|| std::env::var("AURORA_DEMO_SYSTEM_THEME").ok())
        .flatten()
        .filter(|raw| raw.trim() != "system")
        .map(|raw| parse_demo(&raw))
}

/// Whether a demo scenario supplies the looks, which nothing may overwrite;
/// read once, as this is asked every frame.
fn scripted() -> bool {
    static SCRIPTED: OnceLock<bool> = OnceLock::new();
    *SCRIPTED.get_or_init(|| demo_looks().is_some())
}

/// The looks a demo run steps through, as a system would change.
#[derive(Debug, Clone, PartialEq)]
struct DemoLooks {
    looks: Vec<Look>,
    every: Duration,
}

/// `light,accent:#2563EB;dark,accent:#E11D48;secs=3`: the looks in turn,
/// `secs` apart (4 by default); a single look stays.
fn parse_demo(raw: &str) -> DemoLooks {
    let mut demo = DemoLooks {
        looks: Vec::new(),
        every: Duration::from_secs(4),
    };
    for part in raw.split(';') {
        if let Some(secs) = part.trim().strip_prefix("secs=") {
            if let Ok(secs) = secs.parse::<f32>()
                && secs > 0.0
                && let Ok(every) = Duration::try_from_secs_f32(secs)
            {
                demo.every = every;
            }
            continue;
        }
        if part.trim().is_empty() {
            continue;
        }
        let mut look = Look::default();
        for item in part.split(',') {
            match item.trim() {
                "light" => look.scheme = Scheme::Light,
                "dark" => look.scheme = Scheme::Dark,
                other => {
                    if let Some(hex) = other.strip_prefix("accent:") {
                        let [r, g, b, _] = crate::theme::hex(hex).to_array();
                        look.accent = Some([r, g, b]);
                    }
                }
            }
        }
        demo.looks.push(look);
    }
    demo
}

fn run_demo(demo: DemoLooks, ready: std::sync::mpsc::Sender<()>) {
    let mut turn = demo.looks.iter().copied().cycle();
    let single = demo.looks.len() < 2;
    if let Some(look) = turn.next() {
        update(|l| *l = look);
    }
    reported(true);
    let _ = ready.send(());
    if single {
        return;
    }
    loop {
        std::thread::sleep(demo.every);
        if let Some(look) = turn.next() {
            update(|l| *l = look);
        }
    }
}

#[cfg(target_os = "linux")]
mod backend {
    use super::{accent_from_unit, reported, scheme_from_portal, update};
    use std::sync::mpsc::Sender;
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{OwnedValue, Value};

    const DEST: &str = "org.freedesktop.portal.Desktop";
    const PATH: &str = "/org/freedesktop/portal/desktop";
    const SETTINGS: &str = "org.freedesktop.portal.Settings";
    const APPEARANCE: &str = "org.freedesktop.appearance";

    pub(super) fn run(ready: Sender<()>) {
        let proxy = match Connection::session().and_then(|bus| Proxy::new_owned(bus, DEST, PATH, SETTINGS)) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("system theme: no session bus ({e})");
                reported(false);
                return;
            }
        };
        // subscribed before the first reading: a change between the two
        // is not lost
        let signals = match proxy.receive_signal("SettingChanged") {
            Ok(s) => s,
            Err(e) => {
                log::warn!("system theme: changes not followed ({e})");
                reported(false);
                return;
            }
        };
        let mut mode = false;
        for key in ["color-scheme", "accent-color"] {
            match read(&proxy, key) {
                Some(value) => {
                    mode |= key == "color-scheme";
                    apply(key, &value);
                }
                None => log::info!("system theme: the portal has no {key}"),
            }
        }
        reported(mode);
        let _ = ready.send(());
        for msg in signals {
            match msg.body().deserialize::<(String, String, OwnedValue)>() {
                Ok((namespace, key, value)) if namespace == APPEARANCE => apply(&key, &value),
                Ok(_) => {}
                Err(e) => log::debug!("system theme: unreadable change ({e})"),
            }
        }
    }

    /// `ReadOne`, or `Read` on a portal older than its version 2.
    fn read(proxy: &Proxy<'static>, key: &str) -> Option<OwnedValue> {
        let args = (APPEARANCE, key);
        proxy
            .call::<_, _, OwnedValue>("ReadOne", &args)
            .or_else(|_| proxy.call::<_, _, OwnedValue>("Read", &args))
            .ok()
    }

    fn apply(key: &str, value: &Value<'_>) {
        // `Read` wraps the value in one more variant
        let mut value = value;
        while let Value::Value(inner) = value {
            value = inner;
        }
        match key {
            "color-scheme" => {
                if let Ok(n) = u32::try_from(value) {
                    update(|look| look.scheme = scheme_from_portal(n));
                    reported(true);
                }
            }
            "accent-color" => {
                let accent = value
                    .try_clone()
                    .ok()
                    .and_then(|v| <(f64, f64, f64)>::try_from(v).ok())
                    .and_then(|(r, g, b)| accent_from_unit(r, g, b));
                update(|look| look.accent = accent);
            }
            _ => {}
        }
    }

    pub(super) fn tick() {}
}

#[cfg(windows)]
mod backend {
    use super::{active, update};
    use std::sync::mpsc::Sender;
    use std::time::Duration;
    use windows::UI::ViewManagement::{UIColorType, UISettings};
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};

    pub(super) fn run(ready: Sender<()>) {
        // SAFETY: called once at the start of this thread, which owns its
        // apartment; WinRT stays initialized until the thread ends.
        if let Err(e) = unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
            log::warn!("system theme: RoInitialize failed ({e})");
            return;
        }
        let settings = match UISettings::new() {
            Ok(s) => s,
            Err(e) => {
                log::warn!("system theme: no accent colour ({e})");
                return;
            }
        };
        let read = || match settings.GetColorValue(UIColorType::Accent) {
            Ok(c) => update(|look| look.accent = Some([c.R, c.G, c.B])),
            Err(e) => log::debug!("system theme: accent not read ({e})"),
        };
        read();
        let _ = ready.send(());
        loop {
            std::thread::sleep(Duration::from_secs(1));
            if active() {
                read();
            }
        }
    }

    pub(super) fn tick() {}
}

#[cfg(target_os = "macos")]
mod backend {
    use super::{accent_from_unit, update};
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSColor, NSColorSpace};
    use std::cell::Cell;
    use std::sync::mpsc::Sender;
    use std::time::{Duration, Instant};

    thread_local! {
        static LAST: Cell<Option<Instant>> = const { Cell::new(None) };
    }

    /// The accent is read on the main thread (`tick`), like every AppKit
    /// call of the viewer: this thread only has to say it is ready.
    pub(super) fn run(ready: Sender<()>) {
        let _ = ready.send(());
    }

    pub(super) fn tick() {
        if MainThreadMarker::new().is_none() {
            return;
        }
        let due = LAST.get().is_none_or(|t| t.elapsed() >= Duration::from_secs(1));
        if !due {
            return;
        }
        LAST.set(Some(Instant::now()));
        let accent = NSColor::controlAccentColor().colorUsingColorSpace(&NSColorSpace::sRGBColorSpace());
        let Some(accent) = accent else { return };
        let (mut r, mut g, mut b, mut a) = (0.0, 0.0, 0.0, 0.0);
        // SAFETY: four valid pointers to CGFloats that outlive the call, on
        // a colour converted to sRGB, whose components this reads.
        unsafe { accent.getRed_green_blue_alpha(&mut r, &mut g, &mut b, &mut a) };
        if let Some(accent) = accent_from_unit(r, g, b) {
            update(|look| look.accent = Some(accent));
        }
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod backend {
    pub(super) fn run(_ready: std::sync::mpsc::Sender<()>) {}
    pub(super) fn tick() {}
}

#[cfg(test)]
mod tests;
