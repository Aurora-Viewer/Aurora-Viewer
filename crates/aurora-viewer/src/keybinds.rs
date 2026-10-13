//! Rebindable controls (Firestorm "Controls > Shortcuts"): each action
//! has up to two bindings, a keyboard key or a mouse button (middle, back,
//! forward, others) with optional modifiers. Keys are stored by their winit
//! physical code name so the layout does not matter (ZQSD on AZERTY = WASD).

mod layout;

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

/// A physical input: `Key("KeyW")`, `Mouse("Middle")`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Input {
    Key(String),
    Mouse(String),
}

impl Input {
    pub fn key(code: KeyCode) -> Input {
        Input::Key(format!("{code:?}"))
    }

    /// Mouse buttons that can be bound (left / right drive the camera and
    /// the context menu, like in Firestorm).
    pub fn mouse(b: MouseButton) -> Option<Input> {
        match b {
            MouseButton::Left | MouseButton::Right => None,
            MouseButton::Middle => Some(Input::Mouse(aurora_i18n::tr!("keybinds-middle").into())),
            MouseButton::Back => Some(Input::Mouse(aurora_i18n::tr!("keybinds-back").into())),
            MouseButton::Forward => Some(Input::Mouse(aurora_i18n::tr!("keybinds-forward").into())),
            MouseButton::Other(n) => Some(Input::Mouse(format!("Other{n}"))),
        }
    }

    /// Modifier keys qualify ordinary shortcuts; HUD gestures also accept them alone.
    pub fn is_modifier(code: KeyCode) -> bool {
        matches!(
            code,
            KeyCode::ShiftLeft
                | KeyCode::ShiftRight
                | KeyCode::ControlLeft
                | KeyCode::ControlRight
                | KeyCode::AltLeft
                | KeyCode::AltRight
                | KeyCode::SuperLeft
                | KeyCode::SuperRight
        )
    }

    pub fn label(&self) -> String {
        match self {
            Input::Mouse(m) => match m.as_str() {
                "Middle" => aurora_i18n::tr!("keybinds-middle-click").into(),
                "Back" => aurora_i18n::tr!("keybinds-mouse-back").into(),
                "Forward" => aurora_i18n::tr!("keybinds-mouse-forward").into(),
                other => aurora_i18n::tr!("keybinds-mouse", value0 = other.trim_start_matches("Other").to_string()),
            },
            Input::Key(k) => key_label(k),
        }
    }
}

/// Characters seen for physical keys (the real keyboard layout), learned
/// from key events; Windows supplies their labels before the first key press.
static LEARNED: std::sync::RwLock<Option<std::collections::HashMap<String, String>>> = std::sync::RwLock::new(None);

/// Remember the character a physical key produces (unshifted).
pub fn learn(code: KeyCode, text: &str) {
    let t = text.trim();
    if t.is_empty() || t.chars().count() > 2 || t.chars().any(|c| c.is_control()) {
        return;
    }
    if let Ok(mut m) = LEARNED.write() {
        m.get_or_insert_with(Default::default).insert(format!("{code:?}"), t.to_uppercase());
    }
}

fn learned(k: &str) -> Option<String> {
    LEARNED.read().ok()?.as_ref()?.get(k).cloned()
}

fn key_label(k: &str) -> String {
    match k {
        "AltLeft" | "AltRight" => return "ALT".into(),
        "ControlLeft" | "ControlRight" => return "Ctrl".into(),
        "ShiftLeft" | "ShiftRight" => return aurora_i18n::tr!("keybinds-shift").into(),
        _ => {}
    }
    if !k.starts_with(aurora_i18n::tr!("common-numpad"))
        && let Some(l) = learned(k)
    {
        return l;
    }
    if let Some(l) = layout::current().label(k) {
        return l;
    }
    let fixed = match k {
        "ArrowUp" => aurora_i18n::tr!("keybinds-up-arrow"),
        "ArrowDown" => aurora_i18n::tr!("keybinds-down-arrow"),
        "ArrowLeft" => aurora_i18n::tr!("keybinds-left-arrow"),
        "ArrowRight" => aurora_i18n::tr!("keybinds-right-arrow"),
        "PageUp" => aurora_i18n::tr!("keybinds-previous-page"),
        "PageDown" => aurora_i18n::tr!("keybinds-next-page"),
        "Home" => aurora_i18n::tr!("keybinds-origin"),
        "End" => aurora_i18n::tr!("keybinds-end-2"),
        "Insert" => aurora_i18n::tr!("keybinds-insert"),
        "Delete" => aurora_i18n::tr!("keybinds-delete-2"),
        "Backspace" => aurora_i18n::tr!("keybinds-backspace-2"),
        "Enter" => aurora_i18n::tr!("keybinds-enter-2"),
        "NumpadEnter" => aurora_i18n::tr!("keybinds-enter-numpad"),
        "Escape" => aurora_i18n::tr!("keybinds-esc"),
        "Space" => aurora_i18n::tr!("keybinds-space"),
        "Tab" => aurora_i18n::tr!("keybinds-tab"),
        "CapsLock" => aurora_i18n::tr!("keybinds-caps-lock"),
        "Backquote" => "`",
        "Minus" => "-",
        "Equal" => "=",
        "BracketLeft" => "[",
        "BracketRight" => "]",
        "Semicolon" => ";",
        "Quote" => "'",
        "Backslash" => "\\",
        "Comma" => ",",
        "Period" => ".",
        "Slash" => "/",
        "IntlBackslash" => "<",
        "NumpadAdd" => aurora_i18n::tr!("keybinds-numpad-2"),
        "NumpadSubtract" => aurora_i18n::tr!("keybinds-numpad-3"),
        "NumpadMultiply" => aurora_i18n::tr!("keybinds-numpad"),
        "NumpadDivide" => aurora_i18n::tr!("keybinds-numpad-5"),
        "NumpadDecimal" => aurora_i18n::tr!("keybinds-numpad-4"),
        _ => "",
    };
    if !fixed.is_empty() {
        return fixed.into();
    }
    if let Some(c) = k.strip_prefix("Key") {
        return c.into();
    }
    if let Some(d) = k.strip_prefix("Digit") {
        return d.into();
    }
    if let Some(d) = k.strip_prefix(aurora_i18n::tr!("common-numpad")) {
        return aurora_i18n::tr!("keybinds-numpad-6", d = d.to_string());
    }
    k.into()
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Binding {
    pub input: Input,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub alt: bool,
}

impl Binding {
    /// Canonicalize either modifier side without displaying "Ctrl+Ctrl".
    pub fn captured(mut input: Input, mut mods: Mods) -> Binding {
        if let Input::Key(code) = &mut input {
            match code.as_str() {
                "ControlLeft" | "ControlRight" => {
                    *code = "ControlLeft".into();
                    mods.ctrl = false;
                }
                "AltLeft" | "AltRight" => {
                    *code = "AltLeft".into();
                    mods.alt = false;
                }
                "ShiftLeft" | "ShiftRight" => {
                    *code = "ShiftLeft".into();
                    mods.shift = false;
                }
                _ => {}
            }
        }
        Binding {
            input,
            ctrl: mods.ctrl,
            shift: mods.shift,
            alt: mods.alt,
        }
    }

    fn plain(input: Input) -> Binding {
        Binding {
            input,
            ctrl: false,
            shift: false,
            alt: false,
        }
    }

    pub fn label(&self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push_str("Ctrl+");
        }
        if self.alt {
            s.push_str("Alt+");
        }
        if self.shift {
            s.push_str(aurora_i18n::tr!("keybinds-shift-2"));
        }
        s.push_str(&self.input.label());
        s
    }

    /// The keys to hold, modifiers first, for the keycaps of the shortcuts
    /// list.
    pub fn caps(&self) -> Vec<KeyCap> {
        let mut caps = Vec::new();
        if self.ctrl {
            caps.push(KeyCap::named(key_label("ControlLeft")));
        }
        if self.alt {
            caps.push(KeyCap::named("Alt"));
        }
        if self.shift {
            caps.push(KeyCap::named(key_label("ShiftLeft")));
        }
        let mut cap = KeyCap::named(self.input.label());
        cap.mouse = matches!(self.input, Input::Mouse(_));
        let arrow = match &self.input {
            Input::Key(k) => k.strip_prefix("Arrow"),
            Input::Mouse(_) => None,
        };
        if let Some(direction) = arrow {
            cap.icon = Some(format!("square-arrow-{}-filled", direction.to_ascii_lowercase()));
        }
        caps.push(cap);
        caps
    }
}

/// One key of a binding, as the shortcuts list draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyCap {
    pub label: String,
    /// Tabler keycap icon (`square-letter-w-filled`...) when the key has one.
    pub icon: Option<String>,
    /// A mouse button: Tabler has one mouse icon and none per button, so the
    /// cap shows it before the button's name.
    pub mouse: bool,
}

impl KeyCap {
    fn named(label: impl Into<String>) -> KeyCap {
        let label = label.into();
        let icon = key_icon(&label);
        KeyCap { label, icon, mouse: false }
    }
}

/// Tabler draws keycaps for single letters, digits and F1 to F9; the other
/// keys (F10, accented letters, the numeric keypad...) keep their name.
fn key_icon(label: &str) -> Option<String> {
    let mut chars = label.chars();
    match (chars.next(), chars.next(), chars.next()) {
        (Some(c), None, _) if c.is_ascii_alphabetic() => Some(format!("square-letter-{}-filled", c.to_ascii_lowercase())),
        (Some(c), None, _) if c.is_ascii_digit() => Some(format!("square-number-{c}-filled")),
        (Some('F'), Some(digit @ '1'..='9'), None) => Some(format!("square-f{digit}-filled")),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    Forward,
    Back,
    /// Turn (strafe with Shift, or always when "Arrows: strafe").
    Left,
    Right,
    StrafeLeft,
    StrafeRight,
    Up,
    Down,
    Fly,
    AlwaysRun,
    Mouselook,
    ResetCamera,
    /// Firestorm camera keys (spin_around_cw / ccw, spin_over / under,
    /// move_forward / backward, pan_*).
    CamOrbitCw,
    CamOrbitCcw,
    CamOrbitOver,
    CamOrbitUnder,
    CamZoomIn,
    CamZoomOut,
    CamPanLeft,
    CamPanRight,
    CamPanUp,
    CamPanDown,
    Chat,
    PushToTalk,
    ToggleMic,
    Conversations,
    People,
    Inventory,
    Minimap,
    WorldMap,
    /// World › Teleport history (Firestorm Alt+H).
    TeleportHistory,
    /// "Search" (Firestorm Ctrl+F, menu_viewer.xml).
    Search,
    Performance,
    Preferences,
    /// Open Aurora's batch importer and its native file picker.
    Import,
    /// Aurora's held shortcut for dragging a visible HUD with the left button.
    HudDrag,
    /// "Show transparency" (Firestorm Ctrl+Alt+T).
    ShowTransparency,
}

/// Held actions follow the key state; the others trigger on press.
pub fn is_held(a: Action) -> bool {
    matches!(
        a,
        Action::Forward
            | Action::Back
            | Action::Left
            | Action::Right
            | Action::StrafeLeft
            | Action::StrafeRight
            | Action::Up
            | Action::Down
            | Action::PushToTalk
            | Action::CamOrbitCw
            | Action::CamOrbitCcw
            | Action::CamOrbitOver
            | Action::CamOrbitUnder
            | Action::CamZoomIn
            | Action::CamZoomOut
            | Action::CamPanLeft
            | Action::CamPanRight
            | Action::CamPanUp
            | Action::CamPanDown
            | Action::HudDrag
    )
}

/// Actions by section, with their French labels (preferences table order).
pub fn sections() -> &'static Vec<(&'static str, Vec<(Action, &'static str)>)> {
    static TABLE: aurora_i18n::Localized<Vec<(&'static str, Vec<(Action, &'static str)>)>> = aurora_i18n::Localized::new();
    TABLE.get(|| {
        vec![
            (
                aurora_i18n::tr!("common-movement"),
                vec![
                    (Action::Forward, aurora_i18n::tr!("keybinds-move-forward")),
                    (Action::Back, aurora_i18n::tr!("keybinds-move-backward")),
                    (Action::Left, aurora_i18n::tr!("keybinds-turn-left")),
                    (Action::Right, aurora_i18n::tr!("keybinds-turn-right")),
                    (Action::StrafeLeft, aurora_i18n::tr!("keybinds-strafe-left")),
                    (Action::StrafeRight, aurora_i18n::tr!("keybinds-strafe-right")),
                    (Action::Up, aurora_i18n::tr!("extra-jump-ascend")),
                    (Action::Down, aurora_i18n::tr!("status-crouch-descend")),
                    (Action::Fly, aurora_i18n::tr!("extra-fly-land")),
                    (Action::AlwaysRun, aurora_i18n::tr!("keybinds-always-run")),
                ],
            ),
            (
                aurora_i18n::tr!("common-camera"),
                vec![
                    (Action::Mouselook, aurora_i18n::tr!("common-mouselook")),
                    (Action::ResetCamera, aurora_i18n::tr!("keybinds-return-to-avatar")),
                    (Action::CamOrbitCw, aurora_i18n::tr!("keybinds-orbit-clockwise")),
                    (Action::CamOrbitCcw, aurora_i18n::tr!("keybinds-orbit-counterclockwise")),
                    (Action::CamOrbitOver, aurora_i18n::tr!("keybinds-move-above")),
                    (Action::CamOrbitUnder, aurora_i18n::tr!("keybinds-move-below")),
                    (Action::CamZoomIn, aurora_i18n::tr!("keybinds-zoom-camera-in")),
                    (Action::CamZoomOut, aurora_i18n::tr!("keybinds-move-camera-farther-away")),
                    (Action::CamPanLeft, aurora_i18n::tr!("keybinds-pan-view-left")),
                    (Action::CamPanRight, aurora_i18n::tr!("keybinds-pan-view-right")),
                    (Action::CamPanUp, aurora_i18n::tr!("keybinds-pan-view-up")),
                    (Action::CamPanDown, aurora_i18n::tr!("keybinds-pan-view-down")),
                ],
            ),
            (
                aurora_i18n::tr!("common-communication"),
                vec![
                    (Action::Chat, aurora_i18n::tr!("keybinds-type-in-chat")),
                    (Action::PushToTalk, aurora_i18n::tr!("keybinds-speak-push-to-talk")),
                    (Action::ToggleMic, aurora_i18n::tr!("extra-enable-mute-microphone")),
                ],
            ),
            (
                aurora_i18n::tr!("common-windows"),
                vec![
                    (Action::Conversations, aurora_i18n::tr!("common-conversations")),
                    (Action::People, aurora_i18n::tr!("common-people")),
                    (Action::Inventory, aurora_i18n::tr!("common-inventory")),
                    (Action::Minimap, aurora_i18n::tr!("common-mini-map")),
                    (Action::WorldMap, aurora_i18n::tr!("common-world-map")),
                    (Action::TeleportHistory, aurora_i18n::tr!("common-teleport-history")),
                    (Action::Search, aurora_i18n::tr!("search-title")),
                    (Action::Performance, aurora_i18n::tr!("common-performance")),
                    (Action::Preferences, aurora_i18n::tr!("common-preferences")),
                    (Action::Import, aurora_i18n::tr!("keybinds-import-files")),
                ],
            ),
            (
                aurora_i18n::tr!("preferences-group-huds"),
                vec![(Action::HudDrag, aurora_i18n::tr!("ui-options-practical-movement-key"))],
            ),
            (
                aurora_i18n::tr!("keybinds-debug"),
                vec![(Action::ShowTransparency, aurora_i18n::tr!("common-show-transparency"))],
            ),
        ]
    })
}

/// French label of an action.
pub fn label(a: Action) -> &'static str {
    sections()
        .iter()
        .flat_map(|(_, list)| list.iter())
        .find(|(x, _)| *x == a)
        .map(|(_, l)| *l)
        .unwrap_or("?")
}

#[cfg(test)]
pub fn all_actions() -> impl Iterator<Item = Action> {
    sections().iter().flat_map(|(_, list)| list.iter().map(|(a, _)| *a))
}

/// Up to two bindings per action (primary, secondary), as in Firestorm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyBindings {
    pub map: Vec<(Action, [Option<Binding>; 2])>,
}

fn k(code: KeyCode) -> Option<Binding> {
    Some(Binding::plain(Input::key(code)))
}

fn kmod(code: KeyCode, ctrl: bool, shift: bool) -> Option<Binding> {
    Some(Binding {
        input: Input::key(code),
        ctrl,
        shift,
        alt: false,
    })
}

/// Alt (+ Ctrl, + Shift) binding: Firestorm's third-person camera keys.
fn kalt(code: KeyCode, ctrl: bool, shift: bool) -> Option<Binding> {
    Some(Binding {
        input: Input::key(code),
        ctrl,
        shift,
        alt: true,
    })
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self::for_layout(layout::current())
    }
}

impl KeyBindings {
    fn for_layout(layout: &layout::KeyboardLayout) -> Self {
        use KeyCode as K;
        let [forward, back, left, right] = layout.movement_keys();
        let defaults: Vec<(Action, [Option<Binding>; 2])> = vec![
            (Action::Forward, [k(K::ArrowUp), k(forward)]),
            (Action::Back, [k(K::ArrowDown), k(back)]),
            (Action::Left, [k(K::ArrowLeft), k(left)]),
            (Action::Right, [k(K::ArrowRight), k(right)]),
            (Action::StrafeLeft, [None, None]),
            (Action::StrafeRight, [None, None]),
            (Action::Up, [k(K::PageUp), k(K::KeyE)]),
            (Action::Down, [k(K::PageDown), k(K::KeyC)]),
            (Action::Fly, [k(K::Home), k(K::KeyF)]),
            (Action::AlwaysRun, [kmod(K::KeyR, true, false), None]),
            (Action::Mouselook, [k(K::KeyM), None]),
            (Action::ResetCamera, [k(K::Escape), None]),
            (Action::CamOrbitCw, [kalt(K::ArrowLeft, false, false), None]),
            (Action::CamOrbitCcw, [kalt(K::ArrowRight, false, false), None]),
            (Action::CamOrbitOver, [kalt(K::PageUp, false, false), kalt(K::ArrowUp, true, false)]),
            (
                Action::CamOrbitUnder,
                [kalt(K::PageDown, false, false), kalt(K::ArrowDown, true, false)],
            ),
            (Action::CamZoomIn, [kalt(K::ArrowUp, false, false), None]),
            (Action::CamZoomOut, [kalt(K::ArrowDown, false, false), None]),
            (Action::CamPanLeft, [kalt(K::ArrowLeft, true, true), None]),
            (Action::CamPanRight, [kalt(K::ArrowRight, true, true), None]),
            (Action::CamPanUp, [kalt(K::ArrowUp, true, true), None]),
            (Action::CamPanDown, [kalt(K::ArrowDown, true, true), None]),
            (Action::Chat, [k(K::Enter), k(K::NumpadEnter)]),
            // SL default push-to-talk: middle mouse button
            (
                Action::PushToTalk,
                [Some(Binding::plain(Input::Mouse(aurora_i18n::tr!("keybinds-middle").into()))), None],
            ),
            (Action::ToggleMic, [None, None]),
            (Action::Conversations, [kmod(K::KeyT, true, false), None]),
            (Action::People, [kmod(K::KeyA, true, true), None]),
            (Action::Inventory, [kmod(K::KeyI, true, false), None]),
            (Action::Minimap, [kmod(K::KeyM, true, true), None]),
            (Action::WorldMap, [kmod(K::KeyM, true, false), None]),
            (
                Action::TeleportHistory,
                [
                    Some(Binding {
                        input: Input::key(K::KeyH),
                        ctrl: false,
                        shift: false,
                        alt: true,
                    }),
                    None,
                ],
            ),
            (Action::Search, [kmod(K::KeyF, true, false), None]),
            (Action::Performance, [kmod(K::Digit1, true, true), None]),
            (Action::Preferences, [kmod(K::KeyP, true, false), None]),
            (Action::Import, [kmod(K::KeyU, true, false), None]),
            (Action::HudDrag, [k(K::AltLeft), None]),
            (
                Action::ShowTransparency,
                [
                    Some(Binding {
                        input: Input::key(K::KeyT),
                        ctrl: true,
                        shift: false,
                        alt: true,
                    }),
                    None,
                ],
            ),
        ];
        KeyBindings { map: defaults }
    }
}

/// Keys a focused text field uses with Ctrl (select all, copy, cut, paste,
/// undo / redo, word moves and deletes): a viewer shortcut on them never
/// fires while typing. Physical keys, so both the QWERTY and the AZERTY
/// places of A and Z count.
pub fn is_text_edit_key(code: KeyCode) -> bool {
    use KeyCode as K;
    matches!(
        code,
        K::KeyA
            | K::KeyQ
            | K::KeyC
            | K::KeyV
            | K::KeyX
            | K::KeyZ
            | K::KeyW
            | K::KeyY
            | K::ArrowLeft
            | K::ArrowRight
            | K::ArrowUp
            | K::ArrowDown
            | K::Home
            | K::End
            | K::Backspace
            | K::Delete
    )
}

/// Current modifier state.
#[derive(Debug, Clone, Copy, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// Legacy single-key HUD setting, read only to migrate older settings files.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum HoldKey {
    #[default]
    Alt,
    Ctrl,
    Shift,
    Key(String),
}

impl HoldKey {
    pub fn into_binding(self) -> Binding {
        let input = match self {
            Self::Alt => Input::key(KeyCode::AltLeft),
            Self::Ctrl => Input::key(KeyCode::ControlLeft),
            Self::Shift => Input::key(KeyCode::ShiftLeft),
            Self::Key(code) => Input::Key(code),
        };
        Binding::captured(input, Mods::default())
    }
}

impl KeyBindings {
    /// Fill actions missing from an older settings file with their defaults.
    pub fn sanitized(mut self) -> KeyBindings {
        let defaults = KeyBindings::default();
        let mut seen = HashSet::new();
        self.map.retain(|(a, _)| seen.insert(*a));
        for (a, b) in defaults.map {
            if !self.map.iter().any(|(x, _)| *x == a) {
                self.map.push((a, b));
            }
        }
        self
    }

    /// Historical settings-version 4 migration: remove the former letter
    /// defaults. Newly created settings already use the current version.
    pub fn drop_letter_walk(&mut self) {
        let old = [
            (Action::Forward, KeyCode::KeyW),
            (Action::Back, KeyCode::KeyS),
            (Action::Left, KeyCode::KeyA),
            (Action::Right, KeyCode::KeyD),
        ];
        for (a, code) in old {
            let letter = Binding::plain(Input::key(code));
            if let Some((_, binds)) = self.map.iter_mut().find(|(x, _)| *x == a) {
                for b in binds.iter_mut() {
                    if b.as_ref() == Some(&letter) {
                        *b = None;
                    }
                }
                if binds[0].is_none() {
                    binds.swap(0, 1);
                }
            }
        }
    }

    pub fn get(&self, a: Action) -> [Option<Binding>; 2] {
        self.map
            .iter()
            .find(|(x, _)| *x == a)
            .map(|(_, b)| b.clone())
            .unwrap_or([None, None])
    }

    /// Track either gesture slot even while text or media owns keyboard focus.
    pub fn uses_input(&self, a: Action, input: &Input) -> bool {
        self.map
            .iter()
            .any(|(x, bs)| *x == a && bs.iter().flatten().any(|b| &b.input == input))
    }

    pub fn set(&mut self, a: Action, slot: usize, b: Option<Binding>) {
        let slot = slot.min(1);
        if let Some((_, binds)) = self.map.iter_mut().find(|(x, _)| *x == a) {
            binds[slot] = b;
        } else {
            let mut binds = [None, None];
            binds[slot] = b;
            self.map.push((a, binds));
        }
    }

    /// Actions already using this binding (to warn about conflicts).
    pub fn users(&self, b: &Binding) -> Vec<Action> {
        self.map
            .iter()
            .filter(|(_, bs)| bs.iter().flatten().any(|x| x == b))
            .map(|(a, _)| *a)
            .collect()
    }

    /// Is a held action active? A binding needs its modifiers and accepts
    /// more (Shift makes arrows strafe in SL), but the held binding of the
    /// same key with the most matching modifiers wins (LLViewerInput::scanKey:
    /// Alt+← turns the camera, not the avatar).
    pub fn held(&self, a: Action, down: &HashSet<Input>, m: Mods) -> bool {
        let fits = |b: &Binding| (!b.ctrl || m.ctrl) && (!b.shift || m.shift) && (!b.alt || m.alt);
        let weight = |b: &Binding| b.ctrl as u8 + b.shift as u8 + b.alt as u8;
        self.get(a).iter().flatten().any(|b| {
            let pressed = if a == Action::HudDrag {
                match &b.input {
                    Input::Key(code) if matches!(code.as_str(), "ControlLeft" | "ControlRight") => m.ctrl,
                    Input::Key(code) if matches!(code.as_str(), "AltLeft" | "AltRight") => m.alt,
                    Input::Key(code) if matches!(code.as_str(), "ShiftLeft" | "ShiftRight") => m.shift,
                    _ => down.contains(&b.input),
                }
            } else {
                down.contains(&b.input)
            };
            pressed
                && fits(b)
                // HUD ownership is decided on left press over a visible HUD;
                // its binding must not suppress movement or camera elsewhere.
                && (a == Action::HudDrag || !self.map.iter().any(|(x, bs)| {
                    *x != a && *x != Action::HudDrag && is_held(*x) && bs.iter().flatten().any(|o| o.input == b.input && fits(o) && weight(o) > weight(b))
                }))
        })
    }

    /// Action triggered by a press: an exact modifier match wins, then a
    /// binding without modifiers when neither Ctrl nor Alt is held.
    pub fn triggered(&self, input: &Input, m: Mods) -> Option<Action> {
        let exact = self.map.iter().find(|(a, bs)| {
            !is_held(*a)
                && bs
                    .iter()
                    .flatten()
                    .any(|b| &b.input == input && b.ctrl == m.ctrl && b.shift == m.shift && b.alt == m.alt)
        });
        if let Some((a, _)) = exact {
            return Some(*a);
        }
        if m.ctrl || m.alt {
            return None;
        }
        self.map
            .iter()
            .find(|(a, bs)| !is_held(*a) && bs.iter().flatten().any(|b| &b.input == input && !b.ctrl && !b.alt && !b.shift))
            .map(|(a, _)| *a)
    }

    /// The held actions an input belongs to (double-tap running).
    pub fn held_actions_of(&self, input: &Input) -> Vec<Action> {
        self.map
            .iter()
            .filter(|(a, bs)| is_held(*a) && bs.iter().flatten().any(|b| &b.input == input))
            .map(|(a, _)| *a)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_shortcut_migrates_rebinds_and_keeps_explicitly_cleared_slots() {
        let ctrl = Mods {
            ctrl: true,
            ..Default::default()
        };
        let input = Input::key(KeyCode::KeyU);
        let mut bindings = KeyBindings::default();
        assert_eq!(bindings.triggered(&input, ctrl), Some(Action::Import));
        assert_eq!(bindings.triggered(&input, Mods::default()), None);
        bindings.map.retain(|(action, _)| *action != Action::Import);
        let mut bindings = bindings.sanitized();
        assert_eq!(bindings.triggered(&input, ctrl), Some(Action::Import));
        bindings.set(Action::Import, 0, kmod(KeyCode::KeyJ, true, false));
        let saved = serde_json::to_string(&bindings).expect("save bindings");
        let mut restored = serde_json::from_str::<KeyBindings>(&saved).expect("restore bindings").sanitized();
        assert_eq!(restored.triggered(&input, ctrl), None);
        assert_eq!(restored.triggered(&Input::key(KeyCode::KeyJ), ctrl), Some(Action::Import));
        restored.set(Action::Import, 0, None);
        assert_eq!(restored.sanitized().get(Action::Import), [None, None]);
    }

    #[test]
    fn keycap_icons_exist_only_for_letters_digits_and_f1_to_f9() {
        assert_eq!(key_icon("W").as_deref(), Some("square-letter-w-filled"));
        assert_eq!(key_icon("7").as_deref(), Some("square-number-7-filled"));
        assert_eq!(key_icon("F5").as_deref(), Some("square-f5-filled"));
        for label in ["F10", "F0", "É", "&", "Alt", "Ctrl", ""] {
            assert_eq!(key_icon(label), None, "{label}");
        }
        let binding = Binding {
            input: Input::key(KeyCode::ArrowUp),
            ctrl: true,
            shift: false,
            alt: true,
        };
        let caps = binding.caps();
        assert_eq!(caps.len(), 3);
        assert_eq!((caps[0].label.as_str(), caps[1].label.as_str()), ("Ctrl", "Alt"));
        assert!(caps[0].icon.is_none() && caps[1].icon.is_none());
        assert_eq!(caps[2].icon.as_deref(), Some("square-arrow-up-filled"));
        assert!(caps.iter().all(|cap| !cap.mouse));
        let middle = Binding::plain(Input::Mouse("Middle".into())).caps();
        assert!(middle.len() == 1 && middle[0].mouse && middle[0].icon.is_none());
    }

    #[test]
    fn hud_modifier_capture_accepts_either_side_without_duplicate_modifiers() {
        for (left, right, mods) in [
            (
                KeyCode::AltLeft,
                KeyCode::AltRight,
                Mods {
                    alt: true,
                    ..Default::default()
                },
            ),
            (
                KeyCode::ControlLeft,
                KeyCode::ControlRight,
                Mods {
                    ctrl: true,
                    ..Default::default()
                },
            ),
            (
                KeyCode::ShiftLeft,
                KeyCode::ShiftRight,
                Mods {
                    shift: true,
                    ..Default::default()
                },
            ),
        ] {
            for code in [left, right] {
                let b = Binding::captured(Input::key(code), mods);
                assert_eq!(b, Binding::plain(Input::key(left)));
                let mut kb = KeyBindings::default();
                kb.set(Action::HudDrag, 0, Some(b));
                assert!(kb.held(Action::HudDrag, &HashSet::new(), mods));
                assert!(!kb.held(Action::HudDrag, &HashSet::new(), Mods::default()));
            }
        }
    }

    #[test]
    fn either_hud_slot_can_hold_the_gesture_and_cleared_slots_stay_inactive() {
        let mut kb = KeyBindings::default();
        let ctrl = Mods {
            ctrl: true,
            ..Default::default()
        };
        kb.set(Action::HudDrag, 1, Some(Binding::captured(Input::key(KeyCode::KeyB), ctrl)));
        let down = [Input::key(KeyCode::KeyB)].into_iter().collect();
        assert!(kb.held(
            Action::HudDrag,
            &HashSet::new(),
            Mods {
                alt: true,
                ..Default::default()
            }
        ));
        assert!(kb.uses_input(Action::HudDrag, &Input::key(KeyCode::KeyB)));
        assert!(kb.held(Action::HudDrag, &down, ctrl));
        assert!(!kb.held(Action::HudDrag, &down, Mods::default()));
        assert_eq!(kb.triggered(&Input::key(KeyCode::KeyB), ctrl), None);
        kb.set(Action::HudDrag, 0, None);
        kb.set(Action::HudDrag, 1, None);
        assert!(!kb.sanitized().held(Action::HudDrag, &down, Mods { alt: true, ..ctrl }));
    }

    #[test]
    fn hud_shortcuts_warn_of_conflicts_without_suppressing_other_held_actions() {
        let mut kb = KeyBindings::default();
        let ctrl = Mods {
            ctrl: true,
            ..Default::default()
        };
        let input = Input::key(KeyCode::ArrowUp);
        let b = Binding::captured(input.clone(), ctrl);
        kb.set(Action::HudDrag, 0, Some(b.clone()));
        kb.set(Action::PushToTalk, 1, Some(b.clone()));
        assert_eq!(kb.users(&b), vec![Action::PushToTalk, Action::HudDrag]);
        let down = [input].into_iter().collect();
        assert!(kb.held(Action::HudDrag, &down, ctrl));
        assert!(kb.held(Action::PushToTalk, &down, ctrl));
        kb.set(Action::PushToTalk, 1, None);
        assert!(kb.held(Action::Forward, &down, ctrl));
    }

    #[test]
    fn text_edit_keys_stay_with_the_field() {
        // the default window shortcuts work while typing...
        for code in [KeyCode::Digit1, KeyCode::KeyT, KeyCode::KeyI, KeyCode::KeyM, KeyCode::KeyP] {
            assert!(!is_text_edit_key(code), "{code:?}");
        }
        // ...not on the field's own editing keys
        for code in [KeyCode::KeyC, KeyCode::KeyV, KeyCode::KeyZ, KeyCode::KeyQ, KeyCode::ArrowLeft] {
            assert!(is_text_edit_key(code), "{code:?}");
        }
    }

    #[test]
    fn defaults_cover_every_action() {
        let kb = KeyBindings::default();
        for a in all_actions() {
            assert!(kb.map.iter().any(|(x, _)| *x == a), "{a:?}");
        }
    }

    #[test]
    fn the_most_specific_held_binding_wins() {
        let kb = KeyBindings::default();
        let down: HashSet<Input> = [Input::key(KeyCode::ArrowUp)].into_iter().collect();
        let mods = |ctrl, shift, alt| Mods { ctrl, shift, alt };
        assert!(kb.held(Action::Forward, &down, mods(false, false, false)));
        assert!(kb.held(Action::Forward, &down, mods(true, false, false)));
        assert!(!kb.held(Action::Forward, &down, mods(false, false, true)));
        assert!(kb.held(Action::CamZoomIn, &down, mods(false, false, true)));
        assert!(kb.held(Action::CamOrbitOver, &down, mods(true, false, true)));
        assert!(!kb.held(Action::CamZoomIn, &down, mods(true, false, true)));
        assert!(kb.held(Action::CamPanUp, &down, mods(true, true, true)));
        assert!(!kb.held(Action::CamOrbitOver, &down, mods(true, true, true)));
    }

    #[test]
    fn triggers_respect_modifiers() {
        let kb = KeyBindings::default();
        let m = Input::key(KeyCode::KeyM);
        assert_eq!(kb.triggered(&m, Mods::default()), Some(Action::Mouselook));
        assert_eq!(
            kb.triggered(
                &m,
                Mods {
                    ctrl: true,
                    shift: true,
                    alt: false
                }
            ),
            Some(Action::Minimap)
        );
        assert_eq!(
            kb.triggered(
                &m,
                Mods {
                    ctrl: true,
                    shift: false,
                    alt: false
                }
            ),
            Some(Action::WorldMap)
        );
        assert_eq!(
            kb.triggered(
                &m,
                Mods {
                    ctrl: false,
                    shift: true,
                    alt: true
                }
            ),
            None
        );
    }

    #[test]
    fn movement_ignores_shift_and_mouse_ptt() {
        let kb = KeyBindings::default();
        let mut down = HashSet::new();
        down.insert(Input::key(KeyCode::ArrowUp));
        down.insert(Input::Mouse("Middle".into()));
        let shift = Mods {
            shift: true,
            ..Default::default()
        };
        assert!(kb.held(Action::Forward, &down, shift));
        assert!(kb.held(Action::PushToTalk, &down, Mods::default()));
        assert!(!kb.held(Action::Back, &down, shift));
    }

    #[test]
    fn letter_walk_removed_arrows_kept() {
        let mut kb = KeyBindings::default();
        kb.set(Action::Forward, 0, Some(Binding::plain(Input::key(KeyCode::KeyW))));
        kb.set(Action::Forward, 1, Some(Binding::plain(Input::key(KeyCode::ArrowUp))));
        kb.drop_letter_walk();
        assert_eq!(kb.get(Action::Forward), [Some(Binding::plain(Input::key(KeyCode::ArrowUp))), None]);
        assert_eq!(kb.get(Action::Back)[0], Some(Binding::plain(Input::key(KeyCode::ArrowDown))));
    }

    #[test]
    fn old_files_get_new_actions() {
        let mut kb = KeyBindings::default();
        kb.map.retain(|(a, _)| *a != Action::ToggleMic);
        kb.map.push((Action::Forward, [None, None])); // duplicate
        let kb = kb.sanitized();
        assert_eq!(kb.map.iter().filter(|(a, _)| *a == Action::Forward).count(), 1);
        assert!(kb.map.iter().any(|(a, _)| *a == Action::ToggleMic));
    }

    #[test]
    fn fresh_defaults_bind_movement_in_the_secondary_slots() {
        for layout in [layout::KeyboardLayout::wasd(), layout::KeyboardLayout::zqsd(), Default::default()] {
            let kb = KeyBindings::for_layout(&layout);
            for (action, arrow, letter) in [
                (Action::Forward, KeyCode::ArrowUp, KeyCode::KeyW),
                (Action::Back, KeyCode::ArrowDown, KeyCode::KeyS),
                (Action::Left, KeyCode::ArrowLeft, KeyCode::KeyA),
                (Action::Right, KeyCode::ArrowRight, KeyCode::KeyD),
            ] {
                assert_eq!(kb.get(action), [k(arrow), k(letter)]);
                let down = [Input::key(letter)].into_iter().collect();
                assert!(kb.held(action, &down, Mods::default()));
                assert!(kb.held_actions_of(&Input::key(letter)).contains(&action));
            }
        }
    }

    #[test]
    fn saved_custom_and_empty_movement_slots_are_kept() {
        let mut kb = KeyBindings::default();
        kb.set(Action::Forward, 1, k(KeyCode::KeyT));
        kb.set(Action::Left, 1, None);
        let bytes = serde_json::to_vec(&kb).expect("serialize bindings");
        let loaded: KeyBindings = serde_json::from_slice(&bytes).expect("read bindings");
        assert_eq!(loaded.sanitized(), kb);
    }
}
