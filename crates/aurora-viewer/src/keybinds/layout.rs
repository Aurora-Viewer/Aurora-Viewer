//! Initial keyboard labels, movement defaults and shortcut keys from the
//! active keyboard layout (Windows' input locale, the XKB keymap on Linux,
//! the keyboard input source on macOS), rather than the language of the
//! system, and the layout followed while the viewer runs (`Watch`).
//!
//! - Movement: Firestorm's key_bindings.xml uses fixed WASD letters; adapting
//!   those to ZQSD on AZERTY is intentional (the same places under the hand).
//! - The other shortcuts name a letter or a sign ("M", "`"). Firestorm reads
//!   them as Windows virtual keys (LLKeyboardWin32's translate map), which
//!   follow the layout: on AZERTY, M is the key right of L and ` is ù.
//!   `key_for` finds that key here. On Linux the Legacy Viewer names a key by
//!   the character it types (SDL keycodes, LLKeyboardSDL::LLKeyboardSDL,
//!   indra/llwindow/llkeyboardsdl.cpp): see `from_typed`. On macOS it names
//!   a key by its US QWERTY place, Y and Z swapped on QWERTZ: see
//!   `from_us_places`.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, PoisonError, RwLock};
use winit::keyboard::KeyCode;

/// The labels and names of one layout. Its entries are kept sorted by key,
/// so two reads of the same layout compare equal (`Layouts::activate`).
#[derive(Default, Debug, Clone, PartialEq)]
pub(super) struct KeyboardLayout {
    /// The unshifted character each physical key types, for its label.
    labels: Vec<(KeyCode, char)>,
    /// Firestorm's name of each physical key in this layout: the letter or
    /// sign of its Windows virtual key (VK_M is "M", VK_OEM_3 is "`").
    names: Vec<(KeyCode, char)>,
}

const WASD: [KeyCode; 4] = [KeyCode::KeyW, KeyCode::KeyS, KeyCode::KeyA, KeyCode::KeyD];

/// Firestorm's key names, letters then signs, with the key that carries
/// them on a US QWERTY keyboard and their Windows virtual key. Port of the
/// letter and OEM entries of LLKeyboardWin32::LLKeyboardWin32
/// (indra/llwindow/llkeyboardwin32.cpp, originally LGPL 2.1).
const NAMED_KEYS: [(KeyCode, char, u32); 37] = {
    use KeyCode as K;
    [
        (K::KeyA, 'A', 0x41),
        (K::KeyB, 'B', 0x42),
        (K::KeyC, 'C', 0x43),
        (K::KeyD, 'D', 0x44),
        (K::KeyE, 'E', 0x45),
        (K::KeyF, 'F', 0x46),
        (K::KeyG, 'G', 0x47),
        (K::KeyH, 'H', 0x48),
        (K::KeyI, 'I', 0x49),
        (K::KeyJ, 'J', 0x4A),
        (K::KeyK, 'K', 0x4B),
        (K::KeyL, 'L', 0x4C),
        (K::KeyM, 'M', 0x4D),
        (K::KeyN, 'N', 0x4E),
        (K::KeyO, 'O', 0x4F),
        (K::KeyP, 'P', 0x50),
        (K::KeyQ, 'Q', 0x51),
        (K::KeyR, 'R', 0x52),
        (K::KeyS, 'S', 0x53),
        (K::KeyT, 'T', 0x54),
        (K::KeyU, 'U', 0x55),
        (K::KeyV, 'V', 0x56),
        (K::KeyW, 'W', 0x57),
        (K::KeyX, 'X', 0x58),
        (K::KeyY, 'Y', 0x59),
        (K::KeyZ, 'Z', 0x5A),
        (K::Semicolon, ';', 0xBA),    // VK_OEM_1
        (K::Equal, '=', 0xBB),        // VK_OEM_PLUS
        (K::Comma, ',', 0xBC),        // VK_OEM_COMMA
        (K::Minus, '-', 0xBD),        // VK_OEM_MINUS
        (K::Period, '.', 0xBE),       // VK_OEM_PERIOD
        (K::Slash, '/', 0xBF),        // VK_OEM_2
        (K::Backquote, '`', 0xC0),    // VK_OEM_3
        (K::BracketLeft, '[', 0xDB),  // VK_OEM_4
        (K::Backslash, '\\', 0xDC),   // VK_OEM_5
        (K::BracketRight, ']', 0xDD), // VK_OEM_6
        (K::Quote, '\'', 0xDE),       // VK_OEM_7
    ]
};

impl KeyboardLayout {
    fn letters(&self) -> [char; 4] {
        let azerty = WASD
            .iter()
            .zip(['Z', 'S', 'Q', 'D'])
            .all(|(key, letter)| self.labels.contains(&(*key, letter)));
        if azerty { ['Z', 'S', 'Q', 'D'] } else { ['W', 'S', 'A', 'D'] }
    }

    fn mapped_movement(&self) -> Option<[KeyCode; 4]> {
        let mut keys = WASD;
        for (key, letter) in keys.iter_mut().zip(self.letters()) {
            *key = self.labels.iter().find(|(_, c)| *c == letter)?.0;
        }
        Some(keys)
    }

    pub(super) fn movement_keys(&self) -> [KeyCode; 4] {
        self.mapped_movement().unwrap_or(WASD)
    }

    pub(super) fn label(&self, code: &str) -> Option<String> {
        self.labels
            .iter()
            .find(|(key, _)| format!("{key:?}") == code)
            .map(|(_, c)| c.to_string())
    }

    /// The physical key that Windows names `name` in this layout: a letter
    /// (either case) or one of Firestorm's signs (`` ` ``, `-`, `[`…, named
    /// after their US QWERTY key). `None` when the layout is unknown or has
    /// no such key.
    pub(super) fn key_for(&self, name: char) -> Option<KeyCode> {
        let name = name.to_ascii_uppercase();
        self.names.iter().find(|(_, n)| *n == name).map(|(key, _)| *key)
    }

    /// The key that carries, in this layout, the name `us_key` has on a US
    /// QWERTY keyboard (`KeyM` → the M key, `Backquote` → the VK_OEM_3 key),
    /// or `us_key` itself when it has no such name or the layout lacks it.
    pub(super) fn named_key(&self, us_key: KeyCode) -> KeyCode {
        NAMED_KEYS
            .iter()
            .find(|(key, ..)| *key == us_key)
            .and_then(|(_, name, _)| self.key_for(*name))
            .unwrap_or(us_key)
    }

    /// A layout read from the unshifted character each key types, named the
    /// way the Legacy Viewer's Linux build names keys: a key is "M" when it
    /// types m, "`" when it types ` (SDLK_GRAVE). On French AZERTY M is
    /// still the key right of L, as on Windows, but no key types ` unshifted,
    /// so a ` shortcut keeps its US place (Windows' VK_OEM_3 is ù).
    #[cfg(any(target_os = "linux", test))]
    fn from_typed(typed: impl IntoIterator<Item = (KeyCode, char)>) -> Self {
        let mut layout = KeyboardLayout::default();
        for (key, c) in typed {
            let label = label_char(c);
            layout.labels.push((key, label));
            if let Some((_, name, _)) = NAMED_KEYS.iter().find(|(_, name, _)| *name == label) {
                layout.names.push((key, *name));
            }
        }
        layout.sorted()
    }

    /// A layout read from Windows: each key's virtual key in the layout
    /// (`MAPVK_VSC_TO_VK_EX`), which gives its Firestorm name, and what
    /// `MAPVK_VK_TO_CHAR` returns for it, the unshifted character in the low
    /// word (a dead key sets the high bit).
    #[cfg(any(windows, test))]
    fn from_virtual_keys(keys: impl IntoIterator<Item = (KeyCode, u32, u32)>) -> Self {
        let mut layout = KeyboardLayout::default();
        for (key, vk, character) in keys {
            if let Some((_, name, _)) = NAMED_KEYS.iter().find(|(.., named_vk)| *named_vk == vk) {
                layout.names.push((key, *name));
            }
            if let Some(c) = char::from_u32(character & 0xffff).filter(|c| !c.is_control() && !c.is_whitespace()) {
                layout.labels.push((key, label_char(c)));
            }
        }
        layout.sorted()
    }

    /// A layout read from macOS: the unshifted character each key types,
    /// for its label. Names stay at their US QWERTY places, as in the Legacy
    /// Viewer's Mac build (LLKeyboardMacOSX's translate map of virtual key
    /// codes, indra/llwindow/llkeyboardmacosx.cpp), except its one swap: the
    /// US Z key is "Y" when it types y, the US Y key "Z" when it types z
    /// (callKeyDown, indra/llwindow/llwindowmacosx.cpp). On French AZERTY M
    /// stays on the US M key (which types ,), where Windows takes the key
    /// right of L.
    #[cfg(any(target_os = "macos", test))]
    fn from_us_places(typed: impl IntoIterator<Item = (KeyCode, char)>) -> Self {
        let mut layout = KeyboardLayout::default();
        for (key, c) in typed {
            layout.labels.push((key, label_char(c)));
        }
        let types = |key, c: char| layout.labels.iter().any(|&(k, label)| k == key && label == c);
        let swap_y = types(KeyCode::KeyZ, 'Y');
        let swap_z = types(KeyCode::KeyY, 'Z');
        layout.names = NAMED_KEYS
            .iter()
            .map(|&(key, name, _)| match key {
                KeyCode::KeyZ if swap_y => (key, 'Y'),
                KeyCode::KeyY if swap_z => (key, 'Z'),
                _ => (key, name),
            })
            .collect();
        layout.sorted()
    }

    fn sorted(mut self) -> Self {
        self.labels.sort_unstable();
        self.names.sort_unstable();
        self
    }

    /// A layout described by the characters of its keys: `letters` gives what
    /// the 26 letter keys type (KeyA to KeyZ, at their US QWERTY places) and
    /// `others` the other keys, each with its Firestorm name if it has one.
    /// A letter key's virtual key is the character it types (VK_M types m;
    /// AZERTY's "," key is VK_OEM_COMMA).
    fn described(letters: &str, others: &[(KeyCode, char, Option<char>)]) -> Self {
        let mut layout = KeyboardLayout::default();
        for ((key, ..), typed) in NAMED_KEYS.iter().zip(letters.chars()) {
            layout.labels.push((*key, typed));
            if NAMED_KEYS.iter().any(|(_, name, _)| *name == typed) {
                layout.names.push((*key, typed));
            }
        }
        for &(key, typed, name) in others {
            layout.labels.push((key, typed));
            if let Some(name) = name {
                layout.names.push((key, name));
            }
        }
        layout.sorted()
    }

    /// US QWERTY, for the demo and the tests.
    pub(super) fn wasd() -> Self {
        use KeyCode as K;
        Self::described(
            "ABCDEFGHIJKLMNOPQRSTUVWXYZ",
            &[
                (K::Backquote, '`', Some('`')),
                (K::Minus, '-', Some('-')),
                (K::Equal, '=', Some('=')),
                (K::BracketLeft, '[', Some('[')),
                (K::BracketRight, ']', Some(']')),
                (K::Semicolon, ';', Some(';')),
                (K::Quote, '\'', Some('\'')),
                (K::Backslash, '\\', Some('\\')),
                (K::Comma, ',', Some(',')),
                (K::Period, '.', Some('.')),
                (K::Slash, '/', Some('/')),
            ],
        )
    }

    /// French AZERTY (Windows' KBDFR), for the demo and the tests.
    pub(super) fn zqsd() -> Self {
        use KeyCode as K;
        Self::described(
            "QBCDEFGHIJKL,NOPARSTUVZXYW",
            &[
                (K::Backquote, '²', Some('\'')),
                (K::Minus, ')', Some('[')),
                (K::Equal, '=', Some('=')),
                (K::BracketLeft, '^', Some(']')),
                (K::BracketRight, '$', Some(';')),
                (K::Semicolon, 'M', Some('M')),
                (K::Quote, 'Ù', Some('`')),
                (K::Backslash, '*', Some('\\')),
                (K::Comma, ';', Some('.')),
                (K::Period, ':', Some('/')),
                (K::Slash, '!', None), // VK_OEM_8
                (K::IntlBackslash, '<', None),
            ],
        )
    }

    /// German QWERTZ (Windows' KBDGR), for the demo and the tests.
    pub(super) fn qwertz() -> Self {
        use KeyCode as K;
        Self::described(
            "ABCDEFGHIJKLMNOPQRSTUVWXZY",
            &[
                (K::Backquote, '^', Some('\\')),
                (K::Minus, 'ß', Some('[')),
                (K::Equal, '´', Some(']')),
                (K::BracketLeft, 'Ü', Some(';')),
                (K::BracketRight, '+', Some('=')),
                (K::Semicolon, 'Ö', Some('`')),
                (K::Quote, 'Ä', Some('\'')),
                (K::Backslash, '#', Some('/')),
                (K::Comma, ',', Some(',')),
                (K::Period, '.', Some('.')),
                (K::Slash, '-', Some('-')),
                (K::IntlBackslash, '<', None),
            ],
        )
    }
}

/// A key's label: the capital of the character it types (Ù like a learned
/// key), or the character itself when its capital is longer (ß, not SS).
#[cfg(any(windows, target_os = "linux", target_os = "macos", test))]
fn label_char(c: char) -> char {
    let mut upper = c.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(u), None) => u,
        _ => c,
    }
}

/// The spacing sign of an XKB dead keysym, for its label (AZERTY's ^ key
/// types dead_circumflex), as Windows gives it.
#[cfg(any(target_os = "linux", test))]
fn dead_key_char(keysym: u32) -> Option<char> {
    Some(match keysym {
        0xfe50 => '`', // dead_grave
        0xfe51 => '´', // dead_acute
        0xfe52 => '^', // dead_circumflex
        0xfe53 => '~', // dead_tilde
        0xfe57 => '¨', // dead_diaeresis
        0xfe58 => '°', // dead_abovering
        0xfe5b => '¸', // dead_cedilla
        _ => return None,
    })
}

/// The keys whose character a detection reads: the named ones, and the
/// extra key of ISO keyboards.
#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
fn detected_keys() -> impl Iterator<Item = KeyCode> {
    NAMED_KEYS.iter().map(|(key, ..)| *key).chain([KeyCode::IntlBackslash])
}

/// A detected layout, kept only when it carries the four movement letters,
/// and logged.
#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
fn detected(layout: KeyboardLayout) -> Option<KeyboardLayout> {
    layout.mapped_movement()?;
    let found = |name| layout.key_for(name).map_or_else(|| "none".to_owned(), |key| format!("{key:?}"));
    log::info!(
        "keyboard movement defaults: {}; shortcut keys: M on {}, ` on {}",
        layout.letters().iter().collect::<String>(),
        found('M'),
        found('`')
    );
    Some(layout)
}

/// The named key stored as `code` (`"KeyM"`, `"Backquote"`): the only keys
/// `KeyboardLayout::named_key` can move.
pub(super) fn named_code(code: &str) -> Option<KeyCode> {
    NAMED_KEYS.iter().map(|(key, ..)| *key).find(|key| format!("{key:?}") == code)
}

/// The layouts met and the one in use. At start: every layout of the keymap
/// (an XKB keymap can hold several, `us,fr`), or the one active layout
/// (Windows, macOS). Windows, X11 and macOS name the active one at start;
/// Wayland only to a focused window (XWayland too: its group follows the
/// compositor's once it has the focus), so there the first layout stands in
/// until `Watch` hears the active one. A layout a watch reads later joins
/// the list unless it equals one already there.
pub(super) struct Layouts {
    seen: RwLock<Seen>,
    /// The active layout was not reported at start and no report came since.
    guessed: AtomicBool,
}

struct Seen {
    all: Vec<Arc<KeyboardLayout>>,
    active: usize,
}

impl Layouts {
    fn one(layout: KeyboardLayout) -> Self {
        Self::new(vec![layout], 0, false)
    }

    fn new(all: Vec<KeyboardLayout>, active: usize, guessed: bool) -> Self {
        let mut all: Vec<_> = all.into_iter().map(Arc::new).collect();
        if all.is_empty() {
            all.push(Arc::default());
        }
        let active = active.min(all.len() - 1);
        Self {
            seen: RwLock::new(Seen { all, active }),
            guessed: AtomicBool::new(guessed),
        }
    }

    fn current(&self) -> Arc<KeyboardLayout> {
        let seen = self.seen.read().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(&seen.all[seen.active])
    }

    /// `layout` is reported active: the change, if the layout in use
    /// changes.
    fn activate(&self, layout: KeyboardLayout) -> Option<LayoutChange> {
        let guessed = self.guessed.swap(false, Ordering::Relaxed);
        let mut seen = self.seen.write().unwrap_or_else(PoisonError::into_inner);
        let index = match seen.all.iter().position(|known| **known == layout) {
            Some(index) => index,
            None => {
                seen.all.push(Arc::new(layout));
                seen.all.len() - 1
            }
        };
        let before = std::mem::replace(&mut seen.active, index);
        // two layouts of a keymap can read the same (`ru,ua`: both US
        // QWERTY), and the first equal one is found: nothing changes
        (before != index && seen.all[before] != seen.all[index]).then(|| LayoutChange {
            before: Arc::clone(&seen.all[before]),
            after: Arc::clone(&seen.all[index]),
            guessed,
        })
    }
}

/// The layout in use changed while the viewer runs.
pub struct LayoutChange {
    pub(super) before: Arc<KeyboardLayout>,
    pub(super) after: Arc<KeyboardLayout>,
    /// `before` was only the stand-in for a layout not reported yet: the
    /// defaults chosen with it were chosen for the wrong layout.
    pub guessed: bool,
}

fn layouts() -> &'static Layouts {
    static LAYOUTS: OnceLock<Layouts> = OnceLock::new();
    LAYOUTS.get_or_init(|| {
        if std::env::var_os("AURORA_DEMO").is_some() {
            match std::env::var("AURORA_DEMO_KEYBOARD").as_deref() {
                Ok("wasd") => return Layouts::one(KeyboardLayout::wasd()),
                Ok("zqsd") => return Layouts::one(KeyboardLayout::zqsd()),
                Ok("qwertz") => return Layouts::one(KeyboardLayout::qwertz()),
                Ok("fallback") => return Layouts::one(KeyboardLayout::default()),
                _ => {}
            }
        }
        detect().unwrap_or_else(|| {
            log::info!("keyboard layout detection unavailable; using WASD defaults and US QWERTY shortcut keys");
            Layouts::one(KeyboardLayout::default())
        })
    })
}

pub(super) fn current() -> Arc<KeyboardLayout> {
    layouts().current()
}

/// Follows the active layout while the viewer runs: Wayland's keyboard
/// focus, X11's XKB group and keymap, Windows' input locale, macOS' keyboard
/// input source. Each layout it reads goes through `Layouts::activate`.
pub struct Watch {
    /// Dropped after its first failure, which would only repeat.
    inner: Option<OsWatch>,
}

#[cfg(target_os = "linux")]
use linux::OsWatch;
#[cfg(target_os = "macos")]
use macos::OsWatch;
#[cfg(windows)]
use windows::OsWatch;

/// No layout to hear elsewhere: a watch never exists.
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
struct OsWatch(std::convert::Infallible);

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
impl OsWatch {
    fn new(_: &winit::window::Window) -> Option<Self> {
        None
    }

    fn poll(&mut self) -> Result<Option<KeyboardLayout>, String> {
        match self.0 {}
    }

    fn focused(&mut self) {
        match self.0 {}
    }
}

impl Watch {
    pub fn new(window: &winit::window::Window) -> Option<Self> {
        let demo_layout = std::env::var_os("AURORA_DEMO").is_some()
            && matches!(
                std::env::var("AURORA_DEMO_KEYBOARD").as_deref(),
                Ok("wasd" | "zqsd" | "qwertz" | "fallback")
            );
        if demo_layout {
            // a demo layout is not the system's: nothing to follow
            return None;
        }
        OsWatch::new(window).map(|inner| Self { inner: Some(inner) })
    }

    /// The window got the focus: where the system tells no change at focus
    /// (XWayland keeps its group when it already matches), the next poll
    /// reports the layout in use, which settles a stand-in.
    pub fn focused(&mut self) {
        if let Some(inner) = self.inner.as_mut() {
            inner.focused();
        }
    }

    /// The change of layout reported since the last call, if any. Never
    /// waits.
    pub fn poll(&mut self) -> Option<LayoutChange> {
        let layout = match self.inner.as_mut()?.poll() {
            Ok(layout) => layout?,
            Err(e) => {
                log::warn!("keyboard layout: the layout watch failed, the layout in use stays: {e}");
                self.inner = None;
                return None;
            }
        };
        let change = layouts().activate(layout)?;
        super::forget_learned();
        log::info!(
            "keyboard layout: the active layout changed (movement {})",
            change.after.letters().iter().collect::<String>()
        );
        Some(change)
    }
}

#[cfg(target_os = "linux")]
fn detect() -> Option<Layouts> {
    linux::detect()
}

#[cfg(windows)]
fn detect() -> Option<Layouts> {
    windows::detect().map(Layouts::one)
}

#[cfg(target_os = "macos")]
fn detect() -> Option<Layouts> {
    macos::detect().map(Layouts::one)
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn detect() -> Option<Layouts> {
    None
}

#[cfg(test)]
impl KeyboardLayout {
    /// Belgian AZERTY (Windows' KBDBE): French letters, other signs.
    pub(super) fn belgian() -> Self {
        use KeyCode as K;
        Self::described(
            "QBCDEFGHIJKL,NOPARSTUVZXYW",
            &[
                (K::Backquote, '²', Some('\'')),
                (K::Minus, ')', Some('[')),
                (K::Equal, '-', Some('-')),
                (K::BracketLeft, '^', Some(']')),
                (K::BracketRight, '$', Some(';')),
                (K::Semicolon, 'M', Some('M')),
                (K::Quote, 'Ù', Some('`')),
                (K::Backslash, 'µ', Some('\\')),
                (K::Comma, ';', Some('.')),
                (K::Period, ':', Some('/')),
                (K::Slash, '=', Some('=')),
                (K::IntlBackslash, '<', None),
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_labels_are_available_before_any_key_press() {
        let qwerty = KeyboardLayout::wasd();
        let azerty = KeyboardLayout::zqsd();
        assert_eq!(qwerty.letters(), ['W', 'S', 'A', 'D']);
        assert_eq!(azerty.letters(), ['Z', 'S', 'Q', 'D']);
        for (code, qwerty_label, azerty_label) in [("KeyW", "W", "Z"), ("KeyA", "A", "Q"), ("KeyS", "S", "S"), ("KeyD", "D", "D")] {
            assert_eq!(qwerty.label(code).as_deref(), Some(qwerty_label));
            assert_eq!(azerty.label(code).as_deref(), Some(azerty_label));
        }
        assert_eq!(azerty.label("KeyQ").as_deref(), Some("A"));
        assert_eq!(azerty.label("KeyZ").as_deref(), Some("W"));
        // the keys the letter shortcuts land on show their own letter
        assert_eq!(azerty.label("Semicolon").as_deref(), Some("M"));
        assert_eq!(azerty.label("Quote").as_deref(), Some("Ù"));
        assert_eq!(KeyboardLayout::qwertz().label("Semicolon").as_deref(), Some("Ö"));
    }

    #[test]
    fn a_stand_in_layout_gives_way_to_the_reported_one_once() {
        let layouts = Layouts::new(vec![KeyboardLayout::wasd(), KeyboardLayout::zqsd()], 0, true);
        assert_eq!(layouts.current().letters(), ['W', 'S', 'A', 'D']);
        let change = layouts.activate(KeyboardLayout::zqsd()).expect("the second layout is reported");
        assert!(change.guessed);
        assert_eq!(change.before.letters(), ['W', 'S', 'A', 'D']);
        assert_eq!(change.after.letters(), ['Z', 'S', 'Q', 'D']);
        assert_eq!(layouts.current().letters(), ['Z', 'S', 'Q', 'D']);
        assert!(layouts.activate(KeyboardLayout::zqsd()).is_none(), "the same layout again");
        let change = layouts.activate(KeyboardLayout::wasd()).expect("the user switches back");
        assert!(!change.guessed, "only the first report replaces a stand-in");

        // the stand-in confirmed: nothing moves, and a later switch is the user's
        let layouts = Layouts::new(vec![KeyboardLayout::wasd(), KeyboardLayout::zqsd()], 0, true);
        assert!(layouts.activate(KeyboardLayout::wasd()).is_none());
        assert!(!layouts.activate(KeyboardLayout::zqsd()).expect("a switch").guessed);
    }

    /// Windows, X11 and macOS read the active layout at start: a switch
    /// while running brings a layout the start did not know, which joins
    /// the list once and is found again on the next switch.
    #[test]
    fn a_layout_switched_to_while_running_joins_the_list_once() {
        let layouts = Layouts::one(KeyboardLayout::wasd());
        let change = layouts.activate(KeyboardLayout::qwertz()).expect("a switch to German");
        assert!(!change.guessed, "the start layout was reported, not guessed");
        assert_eq!(*change.before, KeyboardLayout::wasd());
        assert_eq!(*change.after, KeyboardLayout::qwertz());
        assert_eq!(layouts.current().key_for('Z'), Some(KeyCode::KeyY));
        assert!(layouts.activate(KeyboardLayout::qwertz()).is_none(), "read again, the same layout");
        assert!(layouts.activate(KeyboardLayout::wasd()).is_some());
        assert!(layouts.activate(KeyboardLayout::qwertz()).is_some());
        assert_eq!(layouts.seen.read().expect("unpoisoned").all.len(), 2);
        // two equal layouts in one keymap (`ru,ua` both read as US QWERTY):
        // the one reported is found under the other's index, nothing changes
        let layouts = Layouts::new(vec![KeyboardLayout::default(), KeyboardLayout::default()], 1, true);
        assert!(layouts.activate(KeyboardLayout::default()).is_none());
        // an empty start (no detection) still has a layout in use
        assert_eq!(*Layouts::new(Vec::new(), 3, false).current(), KeyboardLayout::default());
    }

    /// What Windows' KBDFR gives for each key read (`MapVirtualKeyExW`):
    /// the virtual key, and the character with the dead-key bit on ^.
    #[test]
    fn windows_virtual_keys_give_the_layout() {
        use KeyCode as K;
        let letters = NAMED_KEYS[..26]
            .iter()
            .zip("QBCDEFGHIJKL,NOPARSTUVZXYW".chars())
            .map(|((key, ..), c)| {
                let vk = if c == ',' { 0xBC } else { u32::from(c) };
                (*key, vk, u32::from(c))
            });
        let read = KeyboardLayout::from_virtual_keys(letters.chain([
            (K::Semicolon, 0x4D, u32::from('M')),
            (K::Equal, 0xBB, u32::from('=')),
            (K::Comma, 0xBE, u32::from(';')),
            (K::Minus, 0xDB, u32::from(')')),
            (K::Period, 0xBF, u32::from(':')),
            (K::Slash, 0xDF, u32::from('!')),
            (K::Backquote, 0xDE, u32::from('²')),
            (K::BracketLeft, 0xDD, 0x8000_0000 | u32::from('^')),
            (K::BracketRight, 0xBA, u32::from('$')),
            (K::Backslash, 0xDC, u32::from('*')),
            (K::Quote, 0xC0, u32::from('ù')),
            (K::IntlBackslash, 0xE2, u32::from('<')),
            (K::Digit1, 0x31, 0), // no character: no label
        ]));
        assert_eq!(read, KeyboardLayout::zqsd());
        assert_eq!(read.key_for('M'), Some(K::Semicolon));
        assert_eq!(read.key_for('`'), Some(K::Quote));
        assert_eq!(read.label("BracketLeft").as_deref(), Some("^"), "dead key");
    }

    /// What macOS' French and German input sources type on each key
    /// (`UCKeyTranslate`): labels follow, names keep the US places.
    #[test]
    fn mac_layouts_keep_the_us_places_but_swap_y_and_z() {
        use KeyCode as K;
        let typed = |letters: &str, others: &[(KeyCode, char)]| {
            let letters: Vec<_> = NAMED_KEYS[..26].iter().map(|(key, ..)| *key).zip(letters.chars()).collect();
            KeyboardLayout::from_us_places(letters.into_iter().chain(others.iter().copied()))
        };
        let azerty = typed(
            "qbcdefghijkl,noparstuvzxyw",
            &[(K::Semicolon, 'm'), (K::Quote, 'ù'), (K::Backquote, '@'), (K::BracketLeft, '^')],
        );
        assert_eq!(azerty.letters(), ['Z', 'S', 'Q', 'D']);
        assert_eq!(azerty.movement_keys(), WASD);
        assert_eq!(azerty.label("KeyM").as_deref(), Some(","));
        assert_eq!(azerty.label("Quote").as_deref(), Some("Ù"));
        // the Legacy Viewer's Mac build: M and ` on their US keys, A on the US A key
        assert_eq!(azerty.key_for('M'), Some(K::KeyM));
        assert_eq!(azerty.key_for('`'), Some(K::Backquote));
        assert_eq!(azerty.key_for('A'), Some(K::KeyA));
        assert_eq!(azerty.named_key(K::KeyM), K::KeyM);

        let qwertz = typed("abcdefghijklmnopqrstuvwxzy", &[(K::Minus, 'ß'), (K::Semicolon, 'ö')]);
        assert_eq!(qwertz.key_for('Z'), Some(K::KeyY));
        assert_eq!(qwertz.key_for('Y'), Some(K::KeyZ));
        assert_eq!(qwertz.key_for('M'), Some(K::KeyM));
        assert_eq!(qwertz.key_for('`'), Some(K::Backquote));
        assert_eq!(qwertz.label("Minus").as_deref(), Some("ß"));

        let us = typed("abcdefghijklmnopqrstuvwxyz", &[(K::Backquote, '`')]);
        for (key, name, _) in NAMED_KEYS {
            assert_eq!(us.key_for(name), Some(key), "{name}");
        }
    }

    #[test]
    fn unavailable_or_incomplete_layout_falls_back_to_wasd() {
        assert_eq!(KeyboardLayout::default().movement_keys(), WASD);
        let incomplete = KeyboardLayout {
            labels: vec![(KeyCode::KeyW, 'Z'), (KeyCode::KeyA, 'Q')],
            names: Vec::new(),
        };
        assert_eq!(incomplete.mapped_movement(), None);
        assert_eq!(incomplete.movement_keys(), WASD);
    }

    #[test]
    fn other_layouts_use_the_actual_positions_of_wasd_letters() {
        let layout = KeyboardLayout {
            labels: vec![
                (KeyCode::KeyZ, 'W'),
                (KeyCode::KeyO, 'S'),
                (KeyCode::KeyA, 'A'),
                (KeyCode::KeyH, 'D'),
            ],
            names: Vec::new(),
        };
        assert_eq!(layout.movement_keys(), [KeyCode::KeyZ, KeyCode::KeyO, KeyCode::KeyA, KeyCode::KeyH]);
    }

    #[test]
    fn letters_and_signs_are_found_where_the_layout_puts_them() {
        use KeyCode as K;
        let us = KeyboardLayout::wasd();
        for (key, name, _) in NAMED_KEYS {
            assert_eq!(us.key_for(name), Some(key), "{name}");
            assert_eq!(us.named_key(key), key, "{name}");
        }
        assert_eq!(us.key_for('m'), Some(K::KeyM));

        for azerty in [KeyboardLayout::zqsd(), KeyboardLayout::belgian()] {
            assert_eq!(azerty.key_for('M'), Some(K::Semicolon));
            assert_eq!(azerty.key_for('A'), Some(K::KeyQ));
            assert_eq!(azerty.key_for('Q'), Some(K::KeyA));
            assert_eq!(azerty.key_for('Z'), Some(K::KeyW));
            assert_eq!(azerty.key_for('W'), Some(K::KeyZ));
            assert_eq!(azerty.key_for('`'), Some(K::Quote), "VK_OEM_3 is ù");
            assert_eq!(azerty.named_key(K::KeyM), K::Semicolon);
            assert_eq!(azerty.named_key(K::Backquote), K::Quote);
            for unchanged in [K::KeyE, K::KeyF, K::KeyI, K::KeyT, K::KeyR, K::KeyS] {
                assert_eq!(azerty.named_key(unchanged), unchanged);
            }
        }

        let qwertz = KeyboardLayout::qwertz();
        assert_eq!(qwertz.key_for('Z'), Some(K::KeyY));
        assert_eq!(qwertz.key_for('Y'), Some(K::KeyZ));
        assert_eq!(qwertz.key_for('M'), Some(K::KeyM));
        assert_eq!(qwertz.key_for('`'), Some(K::Semicolon), "VK_OEM_3 is ö");
        assert_eq!(qwertz.named_key(K::Backquote), K::Semicolon);
    }

    #[test]
    fn linux_layouts_name_keys_by_the_character_they_type() {
        use KeyCode as K;
        // French AZERTY ("fr" in XKB) as libxkbcommon reads it, level 1
        let letters = NAMED_KEYS[..26]
            .iter()
            .map(|(key, ..)| *key)
            .zip("qbcdefghijkl,noparstuvzxyw".chars());
        let azerty = KeyboardLayout::from_typed(letters.chain([
            (K::Semicolon, 'm'),
            (K::Equal, '='),
            (K::Comma, ';'),
            (K::Minus, ')'),
            (K::Period, ':'),
            (K::Slash, '!'),
            (K::Backquote, '²'),
            (K::BracketLeft, dead_key_char(0xfe52).expect("dead_circumflex")),
            (K::BracketRight, '$'),
            (K::Backslash, '*'),
            (K::Quote, 'ù'),
            (K::IntlBackslash, '<'),
        ]));
        assert_eq!(azerty.movement_keys(), [K::KeyW, K::KeyS, K::KeyA, K::KeyD]);
        assert_eq!(azerty.letters(), ['Z', 'S', 'Q', 'D']);
        assert_eq!(azerty.label("KeyW").as_deref(), Some("Z"));
        assert_eq!(azerty.label("Quote").as_deref(), Some("Ù"));
        assert_eq!(azerty.label("BracketLeft").as_deref(), Some("^"));
        assert_eq!(azerty.named_key(K::KeyM), K::Semicolon);
        assert_eq!(azerty.key_for(','), Some(K::KeyM));
        assert_eq!(azerty.key_for(';'), Some(K::Comma));
        // no key types ` unshifted: the shortcut stays on the US key (²)
        assert_eq!(azerty.key_for('`'), None);
        assert_eq!(azerty.named_key(K::Backquote), K::Backquote);
        assert_eq!(dead_key_char(u32::from('a')), None);

        // German QWERTZ: Y and Z swap, ß keeps its label
        let letters = NAMED_KEYS[..26]
            .iter()
            .map(|(key, ..)| *key)
            .zip("abcdefghijklmnopqrstuvwxzy".chars());
        let qwertz = KeyboardLayout::from_typed(letters.chain([(K::Minus, 'ß'), (K::Slash, '-')]));
        assert_eq!(qwertz.movement_keys(), WASD);
        assert_eq!(qwertz.key_for('Z'), Some(K::KeyY));
        assert_eq!(qwertz.label("Minus").as_deref(), Some("ß"));
        assert_eq!(qwertz.key_for('-'), Some(K::Slash));
    }

    #[test]
    fn an_unknown_layout_keeps_the_us_places() {
        let unknown = KeyboardLayout::default();
        assert_eq!(unknown.key_for('M'), None);
        assert_eq!(unknown.named_key(KeyCode::KeyM), KeyCode::KeyM);
        assert_eq!(unknown.named_key(KeyCode::Backquote), KeyCode::Backquote);
        // keys without a Firestorm name stay where they are on any layout
        let azerty = KeyboardLayout::zqsd();
        for key in [KeyCode::Digit1, KeyCode::ArrowUp, KeyCode::Escape, KeyCode::IntlBackslash] {
            assert_eq!(azerty.named_key(key), key);
        }
    }
}
