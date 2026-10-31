//! Rebindable controls (Firestorm "Controls > Shortcuts"): each action
//! has up to two bindings, a keyboard key or a mouse button (middle, back,
//! forward, others) with optional modifiers. Keys are stored by their winit
//! physical code name, so a saved binding keeps its key. The defaults are
//! chosen for the active layout (`layout`): movement by place (ZQSD on
//! AZERTY = WASD), letter shortcuts by letter (M is the M key on AZERTY).

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

/// The Legacy Viewer's `start_gesture` (indra/newview/llviewerinput.cpp).
/// Match the character, including Shift on AZERTY and the numeric keypad,
/// rather than the US slash key's physical position (which can type `?`).
pub fn gesture_chat_key(key: &winit::keyboard::Key, mods: Mods) -> bool {
    !mods.ctrl && !mods.alt && matches!(key, winit::keyboard::Key::Character(text) if text == "/")
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
        self.label_on(cfg!(target_os = "macos"))
    }

    /// Shortcut text for one platform. `mac` follows
    /// LLKeyboard::stringFromAccelerator (indra/llwindow/llkeyboard.cpp):
    /// ⌘, or ⌃ when the chord is `use_mac_ctrl` in menu_viewer.xml, then ⌥
    /// and ⇧, with no "+". Other platforms keep "Ctrl+", "Alt+" and "Shift+".
    pub fn label_on(&self, mac: bool) -> String {
        let mut s = String::new();
        if !mac {
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
            return s;
        }
        if self.ctrl {
            s.push_str(if uses_mac_control(self) { "⌃" } else { "⌘" });
        }
        if self.alt {
            s.push('⌥');
        }
        if self.shift {
            s.push('⇧');
        }
        let key = self.input.label();
        // A leading "-" / "=" / "+" would glue to the modifier symbols.
        if (self.ctrl || self.alt || self.shift) && key.starts_with(['-', '=', '+']) {
            s.push(' ');
        }
        s.push_str(&key);
        s
    }

    /// The keys to hold, modifiers first, for the keycaps of the shortcuts
    /// list.
    pub fn caps(&self) -> Vec<KeyCap> {
        self.caps_on(cfg!(target_os = "macos"))
    }

    pub fn caps_on(&self, mac: bool) -> Vec<KeyCap> {
        let mut caps = Vec::new();
        if self.ctrl {
            let name = if !mac {
                key_label("ControlLeft")
            } else if uses_mac_control(self) {
                "⌃".into()
            } else {
                "⌘".into()
            };
            caps.push(KeyCap::named(name));
        }
        if self.alt {
            caps.push(KeyCap::named(if mac { "⌥" } else { "Alt" }));
        }
        if self.shift {
            caps.push(KeyCap::named(if mac { "⇧".into() } else { key_label("ShiftLeft") }));
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
    /// Physical Movelock (Firestorm Ctrl+Alt+P).
    MoveLock,
    MovementEnhancer,
    PassWall,
    /// Sit on the ground, or stand up when sitting (menu_viewer.xml: "Sit
    /// Down" / "Stand Up", both shortcut="alt|shift|S").
    SitGround,
    Mouselook,
    ResetCamera,
    /// LLViewerJoystick::toggleFlycam (menu_viewer.xml, Alt+Shift+F).
    Flycam,
    /// The Controls window, on its Camera tab (Firestorm's View › Camera
    /// Controls).
    CameraControls,
    /// Change the viewing angle (Legacy Viewer View.ZoomIn / Out / Default).
    LensZoomIn,
    LensZoomOut,
    LensZoomDefault,
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
    /// Communicate › Nearby Chat (menu_viewer.xml, `shortcut="control|H"`,
    /// `use_mac_ctrl="true"`, Floater.ToggleOrBringToFront `fs_nearby_chat`).
    NearbyChat,
    People,
    Inventory,
    Gestures,
    Minimap,
    WorldMap,
    /// World › Teleport history (Firestorm Alt+H).
    TeleportHistory,
    /// World › Teleport Home (the Legacy Viewer's Ctrl+Shift+H).
    TeleportHome,
    /// World › Region / Estate (Firestorm's "Region Details", Alt+R).
    RegionEstate,
    /// The compact texture/material library (Ctrl+Space).
    TextureLibrary,
    /// Aurora's IK authoring panel (Ctrl+K).
    IkEdit,
    /// "Search" (Firestorm Ctrl+F, menu_viewer.xml).
    Search,
    Performance,
    Preferences,
    /// Hide or show the interface (Firestorm's View › Show User Interface,
    /// Alt+Shift+U, `View.ToggleUI`).
    ToggleInterface,
    /// The world's hover tips on or off (the Legacy Viewer's Show Tips,
    /// Ctrl+Shift+T, `View.ShowHoverTips`).
    HoverTips,
    /// Open Aurora's batch importer and its native file picker.
    Import,
    /// Aurora's held shortcut for dragging a visible HUD with the left button.
    HudDrag,
    /// HUD attachments shown or hidden (the Legacy Viewer's View › Show HUD
    /// Attachments, Alt+Shift+H, `View.ShowHUDAttachments`).
    ShowHuds,
    /// "Show transparency" (Firestorm Ctrl+Alt+T).
    ShowTransparency,
    Beacons,
    PropertyLines,
    LandOwners,
    HideParticles,
    /// Aurora's Dev Toolbox window (Ctrl+Alt+D).
    DevToolbox,
    /// "Force appearance update (rebake)" (Firestorm Ctrl+Alt+R).
    Rebake,
    /// Avatar health's "Stop avatar animations", the scripts keeping their
    /// permissions (menu_viewer.xml, shortcut="alt|shift|A").
    StopAnimations,
    /// World › Resync Animations (Tools.ResyncAnimations, Ctrl+S).
    ResyncAnimations,
    /// Avatar health's "Refresh attachments" (menu_viewer.xml,
    /// shortcut="alt|shift|R").
    RefreshAttachments,
    /// The photo window ("Snapshot", Firestorm Ctrl+Shift+S).
    PhotoWindow,
    /// "Snapshot to Disk" (Firestorm Ctrl+`): a photo with the settings of
    /// the photo window, without it.
    PhotoToDisk,
    /// World › Environment's fixed times of day and Use Shared Environment
    /// (menu_viewer.xml: `World.EnvSettings` sunrise / noon / sunset /
    /// midnight / region, Ctrl+Shift+U / Y / N / X / J).
    EnvSunrise,
    EnvMidday,
    EnvSunset,
    EnvMidnight,
    EnvShared,
}

impl Action {
    /// The local time of day an environment shortcut picks
    /// (`Panels::time_of_day`: 0 the shared environment, 1 sunrise, 2 noon,
    /// 3 sunset, 4 midnight).
    pub fn time_of_day(self) -> Option<u8> {
        match self {
            Action::EnvShared => Some(0),
            Action::EnvSunrise => Some(1),
            Action::EnvMidday => Some(2),
            Action::EnvSunset => Some(3),
            Action::EnvMidnight => Some(4),
            _ => None,
        }
    }
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
                    (Action::MoveLock, aurora_i18n::tr!("movelock-title")),
                    (Action::MovementEnhancer, aurora_i18n::tr!("movement-title")),
                    (Action::PassWall, aurora_i18n::tr!("movement-pass")),
                    (Action::SitGround, aurora_i18n::tr!("keybinds-sit-ground-stand")),
                ],
            ),
            (
                aurora_i18n::tr!("common-camera"),
                vec![
                    (Action::Mouselook, aurora_i18n::tr!("common-mouselook")),
                    (Action::ResetCamera, aurora_i18n::tr!("keybinds-return-to-avatar")),
                    (Action::Flycam, aurora_i18n::tr!("joystick-flycam")),
                    (Action::CameraControls, aurora_i18n::tr!("controls-camera-title")),
                    (Action::LensZoomIn, aurora_i18n::tr!("keybinds-lens-zoom-in")),
                    (Action::LensZoomOut, aurora_i18n::tr!("keybinds-lens-zoom-out")),
                    (Action::LensZoomDefault, aurora_i18n::tr!("keybinds-lens-zoom-default")),
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
                    (Action::NearbyChat, aurora_i18n::tr!("common-nearby-chat")),
                    (Action::Conversations, aurora_i18n::tr!("common-conversations")),
                    (Action::People, aurora_i18n::tr!("common-people")),
                    (Action::Inventory, aurora_i18n::tr!("common-inventory")),
                    (Action::TextureLibrary, aurora_i18n::tr!("texture-library-title")),
                    (Action::IkEdit, aurora_i18n::tr!("ik-edit-title")),
                    (Action::Gestures, aurora_i18n::tr!("gesture-title")),
                    (Action::Minimap, aurora_i18n::tr!("common-mini-map")),
                    (Action::WorldMap, aurora_i18n::tr!("common-world-map")),
                    (Action::TeleportHistory, aurora_i18n::tr!("common-teleport-history")),
                    (Action::TeleportHome, aurora_i18n::tr!("keybinds-teleport-home")),
                    (Action::RegionEstate, aurora_i18n::tr!("region-title")),
                    (Action::Search, aurora_i18n::tr!("search-title")),
                    (Action::Performance, aurora_i18n::tr!("common-performance")),
                    (Action::Preferences, aurora_i18n::tr!("common-preferences")),
                    (Action::ToggleInterface, aurora_i18n::tr!("keybinds-hide-interface")),
                    (Action::HoverTips, aurora_i18n::tr!("ui-options-hover-tips")),
                    (Action::Import, aurora_i18n::tr!("keybinds-import-files")),
                ],
            ),
            (
                aurora_i18n::tr!("preferences-group-huds"),
                vec![
                    (Action::ShowHuds, aurora_i18n::tr!("keybinds-show-huds")),
                    (Action::HudDrag, aurora_i18n::tr!("ui-options-practical-movement-key")),
                ],
            ),
            (
                aurora_i18n::tr!("common-appearance"),
                vec![
                    (Action::Rebake, aurora_i18n::tr!("ui-context-rebake")),
                    (Action::StopAnimations, aurora_i18n::tr!("avatar-health-stop-animations")),
                    (Action::ResyncAnimations, aurora_i18n::tr!("toolbar-command-resync-animations")),
                    (Action::RefreshAttachments, aurora_i18n::tr!("avatar-health-refresh-attachments")),
                ],
            ),
            (
                aurora_i18n::tr!("common-environment"),
                vec![
                    (Action::EnvSunrise, aurora_i18n::tr!("ui-bars-sunrise")),
                    (Action::EnvMidday, aurora_i18n::tr!("common-midday")),
                    (Action::EnvSunset, aurora_i18n::tr!("ui-bars-sunset")),
                    (Action::EnvMidnight, aurora_i18n::tr!("common-midnight")),
                    (Action::EnvShared, aurora_i18n::tr!("ui-bars-use-shared-environments")),
                ],
            ),
            (
                aurora_i18n::tr!("photo-title"),
                vec![
                    (Action::PhotoWindow, aurora_i18n::tr!("keybinds-photo-window")),
                    (Action::PhotoToDisk, aurora_i18n::tr!("keybinds-photo-to-disk")),
                ],
            ),
            (
                aurora_i18n::tr!("keybinds-debug"),
                vec![
                    (Action::DevToolbox, aurora_i18n::tr!("dev-toolbox-title")),
                    (Action::ShowTransparency, aurora_i18n::tr!("common-show-transparency")),
                    (Action::Beacons, aurora_i18n::tr!("beacons-title")),
                    (Action::PropertyLines, aurora_i18n::tr!("world-property-lines")),
                    (Action::LandOwners, aurora_i18n::tr!("world-land-owners")),
                    (Action::HideParticles, aurora_i18n::tr!("world-hide-particles")),
                ],
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

/// Walking, strafing, jumping and crouching keep the places of the table
/// (layout-aware WASD / ZQSD, E and C beside them): they are chosen for the
/// hand, not for their letter.
fn walks(a: Action) -> bool {
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
    )
}

impl KeyBindings {
    /// The defaults for a keyboard layout. Like Firestorm, a shortcut named
    /// after its letter (M for mouselook, Ctrl+Shift+A for People, Ctrl+`)
    /// is bound to the key that carries that name in the layout: the table
    /// writes it at its US QWERTY place and `layout::named_key` moves it.
    fn for_layout(layout: &layout::KeyboardLayout) -> Self {
        let mut kb = Self::us_places(layout.movement_keys());
        for (action, binds) in &mut kb.map {
            if walks(*action) {
                continue;
            }
            for b in binds.iter_mut().flatten() {
                if let Input::Key(code) = &b.input
                    && let Some(us_key) = layout::named_code(code)
                {
                    b.input = Input::key(layout.named_key(us_key));
                }
            }
        }
        kb
    }

    /// The default table, every shortcut letter at its US QWERTY place.
    fn us_places([forward, back, left, right]: [KeyCode; 4]) -> Self {
        use KeyCode as K;
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
            (Action::MoveLock, [kalt(K::KeyP, true, false), None]),
            (Action::MovementEnhancer, [kalt(K::KeyM, true, false), None]),
            // Ctrl + the current jump key is derived at use, including rebindings.
            (Action::PassWall, [None, None]),
            // menu_viewer.xml: Sit Down / Stand Up, shortcut="alt|shift|S"
            (Action::SitGround, [kalt(K::KeyS, false, true), None]),
            (Action::Mouselook, [k(K::KeyM), None]),
            (Action::ResetCamera, [k(K::Escape), None]),
            (Action::Flycam, [kalt(K::KeyF, false, true), None]),
            (Action::CameraControls, [kmod(K::KeyC, true, true), None]),
            (Action::LensZoomIn, [kmod(K::Digit0, true, false), kmod(K::Numpad0, true, false)]),
            (Action::LensZoomOut, [kmod(K::Digit8, true, false), kmod(K::Numpad8, true, false)]),
            (
                Action::LensZoomDefault,
                [kmod(K::Digit9, true, false), kmod(K::Numpad9, true, false)],
            ),
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
            // menu_viewer.xml: Nearby Chat, shortcut="control|H" use_mac_ctrl="true"
            (Action::NearbyChat, [kmod(K::KeyH, true, false), None]),
            // SL default push-to-talk: middle mouse button
            (
                Action::PushToTalk,
                [Some(Binding::plain(Input::Mouse(aurora_i18n::tr!("keybinds-middle").into()))), None],
            ),
            (Action::ToggleMic, [None, None]),
            (Action::Conversations, [kmod(K::KeyT, true, false), None]),
            (Action::People, [kmod(K::KeyA, true, true), None]),
            (Action::Inventory, [kmod(K::KeyI, true, false), None]),
            (Action::TextureLibrary, [kmod(K::Space, true, false), None]),
            (Action::IkEdit, [kmod(K::KeyK, true, false), None]),
            (Action::Gestures, [kmod(K::KeyG, true, false), None]),
            (Action::Minimap, [kmod(K::KeyM, true, true), None]),
            (Action::WorldMap, [kmod(K::KeyM, true, false), None]),
            // World.TeleportHome, shortcut="control|shift|H" (menu_viewer.xml)
            (Action::TeleportHome, [kmod(K::KeyH, true, true), None]),
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
            // menu_viewer.xml: RegionEstate, shortcut="alt|R"
            (
                Action::RegionEstate,
                [
                    Some(Binding {
                        input: Input::key(K::KeyR),
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
            // menu_viewer.xml: Show User Interface, shortcut="alt|shift|U"
            (Action::ToggleInterface, [kalt(K::KeyU, false, true), None]),
            // menu_viewer.xml: Show Tips, shortcut="control|shift|T"
            (Action::HoverTips, [kmod(K::KeyT, true, true), None]),
            (Action::Import, [kmod(K::KeyU, true, false), None]),
            (
                Action::Beacons,
                [
                    Some(Binding {
                        input: Input::key(K::KeyN),
                        ctrl: true,
                        alt: true,
                        shift: true,
                    }),
                    None,
                ],
            ),
            (
                Action::PropertyLines,
                [
                    Some(Binding {
                        input: Input::key(K::KeyP),
                        ctrl: true,
                        alt: true,
                        shift: true,
                    }),
                    None,
                ],
            ),
            (
                Action::LandOwners,
                [
                    Some(Binding {
                        input: Input::key(K::KeyO),
                        ctrl: true,
                        alt: true,
                        shift: false,
                    }),
                    None,
                ],
            ),
            (
                Action::HideParticles,
                [
                    Some(Binding {
                        input: Input::key(K::Equal),
                        ctrl: true,
                        alt: true,
                        shift: true,
                    }),
                    None,
                ],
            ),
            // menu_viewer.xml: Show HUD Attachments, shortcut="alt|shift|H"
            (Action::ShowHuds, [kalt(K::KeyH, false, true), None]),
            // Aurora's direct HUD gesture must leave Alt to Firestorm's camera (#266).
            (Action::HudDrag, [k(K::ControlLeft), None]),
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
            (
                Action::Rebake,
                [
                    Some(Binding {
                        input: Input::key(K::KeyR),
                        ctrl: true,
                        shift: false,
                        alt: true,
                    }),
                    None,
                ],
            ),
            // Aurora's Dev Toolbox (Advanced › Dev Toolbox)
            (Action::DevToolbox, [kalt(K::KeyD, true, false), None]),
            // menu_viewer.xml: Avatar Health
            (Action::StopAnimations, [kalt(K::KeyA, false, true), None]),
            (Action::ResyncAnimations, [kmod(K::KeyS, true, false), None]),
            (Action::RefreshAttachments, [kalt(K::KeyR, false, true), None]),
            (Action::PhotoWindow, [kmod(K::KeyS, true, true), None]),
            (Action::PhotoToDisk, [kmod(K::Backquote, true, false), None]),
            // menu_viewer.xml: World › Environment
            (Action::EnvSunrise, [kmod(K::KeyU, true, true), None]),
            (Action::EnvMidday, [kmod(K::KeyY, true, true), None]),
            (Action::EnvSunset, [kmod(K::KeyN, true, true), None]),
            (Action::EnvMidnight, [kmod(K::KeyX, true, true), None]),
            (Action::EnvShared, [kmod(K::KeyJ, true, true), None]),
        ];
        KeyBindings { map: defaults }
    }
}

/// Keys that take part in an editing chord. The chord itself is
/// [`text_field_keeps`]: Ctrl+Shift+A is People, not Select All. Only the
/// tests ask for the key alone now.
#[cfg(test)]
pub fn is_text_edit_key(code: KeyCode) -> bool {
    let ctrl = Mods {
        ctrl: true,
        ..Mods::default()
    };
    let shifted = Mods { shift: true, ..ctrl };
    text_field_keeps(code, ctrl) || text_field_keeps(code, shifted)
}

/// Chords a focused text field keeps. Menu accelerators match the mask
/// exactly and run before the field (LLViewerWindow::handleKey,
/// LLMenuItemGL::handleAcceleratorKey): Select All is `control|A`
/// (menu_edit.xml) and People is `control|shift|A` (menu_viewer.xml).
/// Physical keys, so the AZERTY places of A and Z count too. egui's cursor
/// selects all on any Command+A, Shift included; that is not this chord.
/// Arrows, Home, End, Backspace and Delete stay in the field with any
/// modifier: Option+Left / Right moves by word on macOS, and the camera
/// chords on Alt+arrows must not take them.
pub fn text_field_keeps(code: KeyCode, mods: Mods) -> bool {
    if !mods.ctrl && !mods.alt {
        return false;
    }
    use KeyCode as K;
    match code {
        K::KeyA | K::KeyQ | K::KeyC | K::KeyX | K::KeyV | K::KeyY => mods.ctrl && !mods.alt && !mods.shift,
        // Undo, and redo on Ctrl+Shift+Z. AZERTY's Z is the physical W key.
        K::KeyZ | K::KeyW => mods.ctrl && !mods.alt,
        K::ArrowLeft | K::ArrowRight | K::ArrowUp | K::ArrowDown | K::Home | K::End | K::Backspace | K::Delete => true,
        _ => false,
    }
}

/// Keys the chat bar hands to the avatar instead of its text: the arrows,
/// Page Up / Down (jump, crouch) and Home (fly).
const CHAT_BAR_MOVE_KEYS: [KeyCode; 7] = [
    KeyCode::ArrowLeft,
    KeyCode::ArrowRight,
    KeyCode::ArrowUp,
    KeyCode::ArrowDown,
    KeyCode::PageUp,
    KeyCode::PageDown,
    KeyCode::Home,
];

/// A key pressed while the chat bar has the keyboard moves the avatar when
/// the bar is empty, or always with "Arrow keys always move me while in
/// chat"; Ctrl + an arrow stays with the field (word moves). Port of
/// LLViewerWindow::handleKey (indra/newview/llviewerwindow.cpp, FS
/// Communication UI: FSNearbyChat::defaultChatBarHasFocus / IsIdle and
/// ArrowKeysAlwaysMove, originally LGPL 2.1). The Conversations window's
/// field never does, as in Firestorm.
pub fn chat_bar_moves(code: KeyCode, ctrl: bool, bar_empty: bool, always_move: bool) -> bool {
    let arrow = matches!(
        code,
        KeyCode::ArrowLeft | KeyCode::ArrowRight | KeyCode::ArrowUp | KeyCode::ArrowDown
    );
    CHAT_BAR_MOVE_KEYS.contains(&code) && !(arrow && ctrl) && (bar_empty || always_move)
}

/// The held inputs among those (the avatar's movement while typing).
pub fn is_chat_bar_move_input(input: &Input) -> bool {
    CHAT_BAR_MOVE_KEYS.iter().any(|code| *input == Input::key(*code))
}

/// A held input the chat bar hands to the avatar now: as chat_bar_moves,
/// an arrow held with Ctrl stays with the field (word moves, history), so
/// the avatar must not turn or walk while the caret jumps.
pub fn chat_bar_held_moves(input: &Input, ctrl: bool) -> bool {
    let arrow = [KeyCode::ArrowLeft, KeyCode::ArrowRight, KeyCode::ArrowUp, KeyCode::ArrowDown]
        .iter()
        .any(|code| *input == Input::key(*code));
    is_chat_bar_move_input(input) && !(arrow && ctrl)
}

/// Current modifier state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// Control as a shortcut mask. LLKeyboardMacOSX::currentMask
/// (indra/llwindow/llkeyboardmacosx.cpp): the Control key always counts,
/// and Command counts too, except on a mouse event. `command` is false
/// off macOS. The physical Control key stays distinct for clicks, the
/// wheel and the HUD gesture bound to that key.
pub fn control_down(control: bool, command: bool, mouse: bool) -> bool {
    control || (command && !mouse)
}

/// Menu chords labeled with the Mac Control key (`use_mac_ctrl="true"`
/// on a `control|…` shortcut in menu_viewer.xml). macOS reserves the
/// Command form (Hide, Minimize, cycling windows, and the same list).
/// The flag is a label: both keys still run the shortcut when the event
/// arrives (LLMenuItemGL::handleAcceleratorKey keeps only MASK_NORMALKEYS).
/// Matched by the chord, so a rebind keeps the label of the keys it uses.
pub fn uses_mac_control(binding: &Binding) -> bool {
    let Input::Key(key) = &binding.input else {
        return false;
    };
    if !binding.ctrl {
        return false;
    }
    matches!(
        (key.as_str(), binding.alt, binding.shift),
        ("KeyH", false, false) // Nearby Chat, labeled ⌃H. Command+H stays Hide.
        | ("KeyT", false, false) // Conversations
        | ("KeyM", false, false) // World Map; Command+M minimizes
        | ("KeyT", true, false) // Highlight Transparent
        | ("KeyT", true, true) // include Rigged Transparent
        | ("Backquote", false, false) // Snapshot to Disk; Command+` cycles windows
        | ("KeyQ", true, false) // Show Developer Menu
        | ("Digit3", false, true) // Texture Console
        | ("Digit4", false, true) // Debug Console
        | ("Digit9", false, true) // Fast Timers
        | ("Backquote", false, true) // Region Debug Console
    )
}

/// Shortcut text for a fixed menu chord (`"KeyG"`, not the letter alone).
pub fn chord_label(key: &str, ctrl: bool, alt: bool, shift: bool) -> String {
    Binding {
        input: Input::Key(key.to_string()),
        ctrl,
        alt,
        shift,
    }
    .label()
}

/// On macOS, a Control key event is also Command, so text editing and
/// Close Window follow LLKeyboardMacOSX::currentMask for keys. Pointer
/// events and `ModifiersChanged` stay as reported: Command-click is not
/// Control-click.
pub fn adopt_mac_control_keys(events: &mut [egui::Event], mac: bool) {
    if !mac {
        return;
    }
    for event in events {
        if let egui::Event::Key { modifiers, .. } = event
            && modifiers.ctrl
        {
            modifiers.command = true;
        }
    }
}

/// Select All is Command/Ctrl+A with no Shift. egui's cursor treats every
/// Command+A as select all (`CCursorRange::on_key_press`), which would
/// swallow People (Command+Shift+A) inside a text field.
pub fn keep_select_all_exact(events: &mut [egui::Event]) {
    for event in events {
        if let egui::Event::Key {
            key: egui::Key::A,
            modifiers,
            ..
        } = event
            && modifiers.command
            && modifiers.shift
        {
            modifiers.command = false;
            modifiers.mac_cmd = false;
        }
    }
}

/// Editing history takes priority over held movement and camera bindings:
/// LLViewerInput::scanKey (indra/newview/llviewerinput.cpp), LLEditUndo /
/// LLEditRedo (indra/newview/llviewermenu.cpp). Text and media keep their
/// own history; Ctrl+Shift+Z also matches egui's native redo shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditHistory {
    Undo,
    Redo,
}

impl EditHistory {
    pub fn from_key(key: egui::Key, mods: Mods) -> Option<Self> {
        if !mods.ctrl || mods.alt {
            return None;
        }
        match (key, mods.shift) {
            (egui::Key::Z, false) => Some(Self::Undo),
            (egui::Key::Y, false) | (egui::Key::Z, true) => Some(Self::Redo),
            _ => None,
        }
    }
}

/// The same logical letters/punctuation and non-Latin physical fallback as
/// egui-winit, restricted to keys used by the fixed editing accelerators.
pub fn native_shortcut_key(key: &winit::keyboard::Key, code: KeyCode) -> Option<egui::Key> {
    use egui::Key as E;
    use winit::keyboard::{Key, KeyCode as K, NamedKey};
    let logical = match key {
        Key::Character(letter) => E::from_name(letter),
        Key::Named(NamedKey::Escape) => Some(E::Escape),
        Key::Named(NamedKey::Delete) => Some(E::Delete),
        Key::Named(_) => return None,
        _ => None,
    };
    logical.or(match code {
        K::KeyB => Some(E::B),
        K::KeyD => Some(E::D),
        K::KeyE => Some(E::E),
        K::KeyG => Some(E::G),
        K::KeyL => Some(E::L),
        K::KeyW => Some(E::W),
        K::KeyX => Some(E::X),
        K::KeyZ => Some(E::Z),
        K::KeyY => Some(E::Y),
        K::Digit1 | K::Numpad1 => Some(E::Num1),
        K::Digit2 | K::Numpad2 => Some(E::Num2),
        K::Digit3 | K::Numpad3 => Some(E::Num3),
        K::Digit4 | K::Numpad4 => Some(E::Num4),
        K::Digit5 | K::Numpad5 => Some(E::Num5),
        K::Period | K::NumpadDecimal => Some(E::Period),
        K::Comma => Some(E::Comma),
        K::Escape => Some(E::Escape),
        K::Delete => Some(E::Delete),
        _ => None,
    })
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
        if self.get(Action::PassWall) == [kalt(KeyCode::KeyG, true, false), None] {
            self.set(Action::PassWall, 0, None);
        }
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

    /// Settings-version 12 migration: a shortcut still on its former default,
    /// the US QWERTY place of its letter, moves to the key of that letter in
    /// the active layout (`for_layout`). A slot the user changed or cleared
    /// stays, and so does a default whose new key another action already uses.
    pub fn follow_layout_letters(&mut self) {
        self.follow_letters_of(layout::current());
    }

    fn follow_letters_of(&mut self, layout: &layout::KeyboardLayout) {
        let before = Self::us_places(layout.movement_keys());
        let after = Self::for_layout(layout);
        for ((action, old), (_, new)) in before.map.iter().zip(&after.map) {
            for slot in 0..2 {
                let (Some(old), Some(new)) = (&old[slot], &new[slot]) else {
                    continue;
                };
                if old == new || self.get(*action)[slot].as_ref() != Some(old) || self.users(new).iter().any(|a| a != action) {
                    continue;
                }
                self.set(*action, slot, Some(new.clone()));
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
                && self.triggered(&b.input, m).is_none()
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

    /// A menu/window action owns the press, including a key held before
    /// its modifiers arrived. Never start a walk tap or resume movement
    /// when those modifiers are released before the letter.
    pub fn update_held_input(&self, down: &mut HashSet<Input>, input: &Input, pressed: bool, mods: Mods) -> Option<Action> {
        let command = pressed.then(|| self.triggered(input, mods)).flatten();
        if pressed && command.is_none() {
            down.insert(input.clone());
        } else {
            down.remove(input);
        }
        command
    }

    /// Follow the actual jump bindings; explicit action or camera bindings win.
    pub fn pass_wall_triggered(&self, input: &Input, m: Mods) -> bool {
        if self.get(Action::PassWall).iter().any(Option::is_some) {
            return self.triggered(input, m) == Some(Action::PassWall);
        }
        m.ctrl
            && !m.alt
            && !m.shift
            && self.triggered(input, m).is_none()
            && self
                .get(Action::Up)
                .iter()
                .flatten()
                .any(|b| &b.input == input && !b.ctrl && !b.alt && !b.shift)
            && !self.map.iter().any(|(a, bs)| {
                *a != Action::Up && *a != Action::HudDrag && bs.iter().flatten().any(|b| &b.input == input && b.ctrl && !b.alt && !b.shift)
            })
    }

    /// Explicit passage bindings, or Ctrl plus unmodified jump bindings.
    pub fn pass_wall_bindings(&self) -> [Option<Binding>; 2] {
        let explicit = self.get(Action::PassWall);
        if explicit.iter().any(Option::is_some) {
            return explicit;
        }
        self.get(Action::Up).map(|binding| {
            binding.filter(|b| !b.ctrl && !b.alt && !b.shift).map(|mut b| {
                b.ctrl = true;
                b
            })
        })
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
    use crate::build::ui::Shortcut;

    #[test]
    fn gesture_chat_uses_the_typed_slash_without_ctrl_or_alt() {
        use winit::keyboard::Key;
        for shift in [false, true] {
            let mods = Mods {
                shift,
                ..Default::default()
            };
            assert!(gesture_chat_key(&Key::Character("/".into()), mods));
            for text in ["?", ":", "!", "m", "／"] {
                assert!(!gesture_chat_key(&Key::Character(text.into()), mods), "{text}");
            }
            assert!(!gesture_chat_key(&Key::Named(winit::keyboard::NamedKey::Enter), mods));
            for (ctrl, alt) in [(true, false), (false, true), (true, true)] {
                assert!(!gesture_chat_key(&Key::Character("/".into()), Mods { ctrl, alt, shift }));
            }
        }
    }
    #[test]
    fn history_keys_never_reach_avatar_camera_or_other_held_bindings_in_any_layout() {
        for (code, letter, history) in [
            (KeyCode::KeyZ, "z", EditHistory::Undo),
            (KeyCode::KeyW, "z", EditHistory::Undo), // AZERTY
            (KeyCode::KeyY, "z", EditHistory::Undo), // QWERTZ
            (KeyCode::KeyY, "Y", EditHistory::Redo),
            (KeyCode::KeyZ, "y", EditHistory::Redo), // QWERTZ
            (KeyCode::KeyZ, "я", EditHistory::Undo), // egui's non-Latin fallback
        ] {
            let input = Input::key(code);
            let key = winit::keyboard::Key::Character(letter.into());
            let ctrl = Mods {
                ctrl: true,
                ..Default::default()
            };
            let mut bindings = KeyBindings::default();
            for action in [Action::Forward, Action::CamZoomIn, Action::PushToTalk, Action::HudDrag] {
                bindings.set(action, 0, Some(Binding::plain(input.clone())));
            }
            let other = Input::key(KeyCode::ArrowDown);
            // Also cancel a key already held before Ctrl or a repeat arrives.
            let mut down = HashSet::from([input.clone(), other.clone()]);
            for _ in 0..3 {
                assert_eq!(
                    Shortcut::take_native_key(&key, code, ctrl, false, false, &mut down),
                    Some(Shortcut::History(history))
                );
                assert_eq!(down, HashSet::from([other.clone()]));
                for mods in [ctrl, Mods::default()] {
                    for action in [Action::Forward, Action::CamZoomIn, Action::PushToTalk, Action::HudDrag] {
                        assert!(!bindings.held(action, &down, mods), "{letter}: {action:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn history_reservation_keeps_plain_movement_altgr_and_other_letters() {
        let input = Input::key(KeyCode::KeyW);
        let z = winit::keyboard::Key::Character("z".into());
        for mods in [
            Mods::default(),
            Mods {
                ctrl: true,
                alt: true,
                shift: false,
            },
        ] {
            let mut down = HashSet::from([input.clone()]);
            assert_eq!(Shortcut::take_native_key(&z, KeyCode::KeyW, mods, false, false, &mut down), None);
            assert!(down.contains(&input));
        }
        let mut down = HashSet::from([Input::key(KeyCode::KeyZ)]);
        assert_eq!(
            Shortcut::take_native_key(
                &winit::keyboard::Key::Character("w".into()),
                KeyCode::KeyZ,
                Mods {
                    ctrl: true,
                    ..Default::default()
                },
                false,
                false,
                &mut down,
            ),
            None,
            "AZERTY Ctrl+W is not Ctrl+Z"
        );
        assert!(down.contains(&Input::key(KeyCode::KeyZ)));
        assert_eq!(
            EditHistory::from_key(
                egui::Key::Z,
                Mods {
                    ctrl: true,
                    shift: true,
                    alt: false
                }
            ),
            Some(EditHistory::Redo)
        );
    }

    #[test]
    fn viewer_window_commands_do_not_hold_walk_jump_camera_or_microphone_inputs() {
        let kb = KeyBindings::us_places([KeyCode::KeyW, KeyCode::KeyS, KeyCode::KeyA, KeyCode::KeyD]);
        let mods = Mods {
            ctrl: true,
            shift: true,
            alt: false,
        };
        for (code, action, movement) in [
            (KeyCode::KeyS, Action::PhotoWindow, Action::Back),
            (KeyCode::KeyC, Action::CameraControls, Action::Down),
            (KeyCode::KeyA, Action::People, Action::Left),
        ] {
            let input = Input::key(code);
            let mut down = HashSet::from([input.clone()]);
            assert!(!kb.held(movement, &down, mods), "modifier changes already reserve the command");
            assert_eq!(kb.update_held_input(&mut down, &input, true, mods), Some(action));
            assert!(down.is_empty());
            assert!(
                !kb.held(movement, &down, Mods::default()),
                "releasing Ctrl first cannot resume walking"
            );
            assert_eq!(kb.update_held_input(&mut down, &input, false, Mods::default()), None);
            assert!(down.is_empty());
            assert_eq!(kb.update_held_input(&mut down, &input, true, Mods::default()), None);
            assert!(kb.held(movement, &down, Mods::default()), "a new ordinary press still moves");
        }
        let mut kb = kb;
        let input = Input::key(KeyCode::KeyT);
        for action in [Action::Forward, Action::CamZoomIn, Action::PushToTalk, Action::HudDrag] {
            kb.set(action, 0, Some(Binding::plain(input.clone())));
        }
        let mut down = HashSet::from([input.clone()]);
        let ctrl = Mods {
            ctrl: true,
            ..Default::default()
        };
        assert_eq!(kb.update_held_input(&mut down, &input, true, ctrl), Some(Action::Conversations));
        for action in [Action::Forward, Action::CamZoomIn, Action::PushToTalk, Action::HudDrag] {
            assert!(!kb.held(action, &down, ctrl));
            assert!(!kb.held(action, &down, Mods::default()));
        }
    }

    #[test]
    fn held_camera_combinations_keep_their_own_input_after_command_reservation() {
        let kb = KeyBindings::default();
        let input = Input::key(KeyCode::ArrowUp);
        for (mods, action) in [
            (
                Mods {
                    alt: true,
                    ..Default::default()
                },
                Action::CamZoomIn,
            ),
            (
                Mods {
                    alt: true,
                    ctrl: true,
                    shift: false,
                },
                Action::CamOrbitOver,
            ),
            (
                Mods {
                    alt: true,
                    ctrl: true,
                    shift: true,
                },
                Action::CamPanUp,
            ),
        ] {
            let mut down = HashSet::new();
            assert_eq!(kb.update_held_input(&mut down, &input, true, mods), None);
            assert!(kb.held(action, &down, mods));
            assert!(!kb.held(Action::Forward, &down, mods));
            let _ = kb.update_held_input(&mut down, &input, false, mods);
            assert!(!kb.held(action, &down, mods));
        }
    }

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
        let caps = binding.caps_on(false);
        assert_eq!(caps.len(), 3);
        assert_eq!((caps[0].label.as_str(), caps[1].label.as_str()), ("Ctrl", "Alt"));
        assert!(caps[0].icon.is_none() && caps[1].icon.is_none());
        assert_eq!(caps[2].icon.as_deref(), Some("square-arrow-up-filled"));
        assert!(caps.iter().all(|cap| !cap.mouse));
        let mac = binding.caps_on(true);
        assert_eq!((mac[0].label.as_str(), mac[1].label.as_str()), ("⌘", "⌥"));
        let middle = Binding::plain(Input::Mouse("Middle".into())).caps_on(false);
        assert!(middle.len() == 1 && middle[0].mouse && middle[0].icon.is_none());
    }

    #[test]
    fn nearby_chat_is_ctrl_h_and_keeps_the_other_h_chords() {
        let kb = KeyBindings::default();
        let h = Input::key(KeyCode::KeyH);
        let ctrl = Mods {
            ctrl: true,
            ..Default::default()
        };
        let alt = Mods {
            alt: true,
            ..Default::default()
        };
        let alt_shift = Mods {
            alt: true,
            shift: true,
            ..Default::default()
        };
        assert_eq!(kb.triggered(&h, ctrl), Some(Action::NearbyChat));
        assert_eq!(kb.triggered(&h, alt), Some(Action::TeleportHistory));
        assert_eq!(kb.triggered(&h, alt_shift), Some(Action::ShowHuds));
        assert_eq!(kb.triggered(&h, Mods::default()), None);
        assert!(!text_field_keeps(KeyCode::KeyH, ctrl), "Nearby Chat runs while a field is focused");
        let binding = kb.get(Action::NearbyChat)[0].clone().expect("default binding");
        assert_eq!(binding.label_on(true), "⌃H");
        assert_eq!(binding.label_on(false), "Ctrl+H");
        let mut old = kb;
        old.map.retain(|(action, _)| *action != Action::NearbyChat);
        assert_eq!(old.sanitized().triggered(&h, ctrl), Some(Action::NearbyChat));
    }

    #[test]
    fn mac_shortcuts_label_command_and_the_legacy_control_chords() {
        let chord = |key, alt, shift| Binding {
            input: Input::key(key),
            ctrl: true,
            alt,
            shift,
        };
        let world = chord(KeyCode::KeyM, false, false);
        let minimap = chord(KeyCode::KeyM, false, true);
        let close = chord(KeyCode::KeyW, false, false);
        let photo = chord(KeyCode::Backquote, false, false);
        let transparent = chord(KeyCode::KeyT, true, false);
        let conversations = chord(KeyCode::KeyT, false, false);
        // the key itself reads as on this PC's layout (M is "," in AZERTY)
        let key = |code| Input::key(code).label();
        let m = key(KeyCode::KeyM);
        assert_eq!(world.label_on(false), format!("Ctrl+{m}"));
        assert_eq!(world.label_on(true), format!("⌃{m}"));
        assert_eq!(minimap.label_on(true), format!("⌘⇧{m}"));
        assert_eq!(close.label_on(true), format!("⌘{}", key(KeyCode::KeyW)));
        assert_eq!(photo.label_on(true), format!("⌃{}", key(KeyCode::Backquote)));
        assert_eq!(transparent.label_on(true), format!("⌃⌥{}", key(KeyCode::KeyT)));
        assert_eq!(conversations.label_on(true), format!("⌃{}", key(KeyCode::KeyT)));
        let minus = key(KeyCode::Minus);
        let gap = if minus.starts_with(['-', '=', '+']) { " " } else { "" };
        assert_eq!(chord(KeyCode::Minus, false, false).label_on(true), format!("⌘{gap}{minus}"));
        assert!(!uses_mac_control(&minimap));
        assert!(uses_mac_control(&world));
        assert!(!control_down(false, true, true), "Command is not Control for the mouse");
        assert!(control_down(false, true, false));
        assert!(control_down(true, false, true));
        assert!(!control_down(false, false, false));
        let mut events = [
            egui::Event::Key {
                key: egui::Key::C,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::CTRL,
            },
            egui::Event::Key {
                key: egui::Key::W,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
            },
            egui::Event::ModifiersChanged(egui::Modifiers::CTRL),
        ];
        adopt_mac_control_keys(&mut events, false);
        assert!(matches!(events[0], egui::Event::Key { modifiers, .. } if !modifiers.command));
        adopt_mac_control_keys(&mut events, true);
        assert!(matches!(events[0], egui::Event::Key { modifiers, .. } if modifiers.command && modifiers.ctrl));
        assert!(matches!(
            events[1],
            egui::Event::Key { modifiers, .. } if modifiers.command && modifiers.shift && !modifiers.matches_exact(egui::Modifiers::COMMAND)
        ));
        assert!(matches!(events[2], egui::Event::ModifiersChanged(m) if m == egui::Modifiers::CTRL));
        events[0] = egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
        };
        adopt_mac_control_keys(&mut events, true);
        keep_select_all_exact(&mut events);
        assert!(matches!(
            events[0],
            egui::Event::Key { modifiers, .. } if !modifiers.command && modifiers.shift && modifiers.ctrl
        ));
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
    fn default_hud_gesture_leaves_alt_and_shift_alt_to_the_camera() {
        let kb = KeyBindings::default();
        let down = HashSet::new();
        for shift in [false, true] {
            assert!(!kb.held(
                Action::HudDrag,
                &down,
                Mods {
                    alt: true,
                    shift,
                    ctrl: false
                }
            ));
            assert!(kb.held(
                Action::HudDrag,
                &down,
                Mods {
                    ctrl: true,
                    shift,
                    alt: false
                }
            ));
        }
    }

    #[test]
    fn either_hud_slot_can_hold_the_gesture_and_cleared_slots_stay_inactive() {
        let mut kb = KeyBindings::default();
        kb.set(Action::HudDrag, 0, Some(HoldKey::Alt.into_binding()));
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
        let ctrl = Mods {
            ctrl: true,
            ..Mods::default()
        };
        let shifted = Mods { shift: true, ..ctrl };
        assert!(text_field_keeps(KeyCode::KeyA, ctrl), "Select All");
        assert!(!text_field_keeps(KeyCode::KeyA, shifted), "People is Ctrl+Shift+A");
        assert!(!text_field_keeps(KeyCode::KeyQ, shifted), "the same chord on AZERTY");
        assert!(!text_field_keeps(KeyCode::KeyY, shifted), "Ctrl+Shift+Y is not redo");
        assert!(text_field_keeps(KeyCode::KeyZ, shifted));
        let alt = Mods {
            alt: true,
            ..Mods::default()
        };
        assert!(text_field_keeps(KeyCode::ArrowLeft, alt), "Option+Left stays in the field");
        assert!(
            text_field_keeps(
                KeyCode::ArrowUp,
                Mods {
                    ctrl: true,
                    alt: true,
                    shift: true,
                }
            ),
            "Ctrl+Alt+Shift+Up stays in the field"
        );
    }

    /// LLViewerWindow::handleKey: the chat bar hands the arrows, Page Up /
    /// Down and Home to the avatar when empty or with ArrowKeysAlwaysMove,
    /// keeps Ctrl + arrows and every other key.
    #[test]
    fn chat_bar_hands_movement_keys_to_the_avatar() {
        let moving = [
            KeyCode::ArrowLeft,
            KeyCode::ArrowRight,
            KeyCode::ArrowUp,
            KeyCode::ArrowDown,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Home,
        ];
        for code in moving {
            assert!(chat_bar_moves(code, false, true, false), "{code:?} with an empty bar");
            assert!(chat_bar_moves(code, false, false, true), "{code:?} with text, always move");
            assert!(!chat_bar_moves(code, false, false, false), "{code:?} with text stays in it");
            assert!(is_chat_bar_move_input(&Input::key(code)));
        }
        for code in [KeyCode::ArrowLeft, KeyCode::ArrowRight, KeyCode::ArrowUp, KeyCode::ArrowDown] {
            assert!(!chat_bar_moves(code, true, true, true), "Ctrl + {code:?} stays with the field");
        }
        assert!(chat_bar_moves(KeyCode::PageUp, true, true, true));
        for code in [KeyCode::KeyW, KeyCode::End, KeyCode::Enter, KeyCode::Space] {
            assert!(!chat_bar_moves(code, false, true, true), "{code:?}");
            assert!(!is_chat_bar_move_input(&Input::key(code)));
        }
    }

    /// While the caret jumps with Ctrl + an arrow, the held arrow does not
    /// also turn or walk the avatar; Page Up / Down still pass.
    #[test]
    fn ctrl_arrows_held_in_the_chat_bar_stay_with_the_field() {
        for code in [KeyCode::ArrowLeft, KeyCode::ArrowRight, KeyCode::ArrowUp, KeyCode::ArrowDown] {
            assert!(chat_bar_held_moves(&Input::key(code), false), "{code:?}");
            assert!(!chat_bar_held_moves(&Input::key(code), true), "Ctrl + {code:?}");
        }
        assert!(chat_bar_held_moves(&Input::key(KeyCode::PageUp), true));
        assert!(!chat_bar_held_moves(&Input::key(KeyCode::KeyW), false));
    }

    #[test]
    fn lens_shortcuts_are_added_to_old_settings_and_can_be_rebound() {
        let mut kb = KeyBindings::default();
        kb.map
            .retain(|(a, _)| !matches!(a, Action::LensZoomIn | Action::LensZoomOut | Action::LensZoomDefault));
        let mut kb = kb.sanitized();
        let ctrl = Mods {
            ctrl: true,
            ..Default::default()
        };
        for (row, pad, action) in [
            (KeyCode::Digit0, KeyCode::Numpad0, Action::LensZoomIn),
            (KeyCode::Digit8, KeyCode::Numpad8, Action::LensZoomOut),
            (KeyCode::Digit9, KeyCode::Numpad9, Action::LensZoomDefault),
        ] {
            assert_eq!(kb.triggered(&Input::key(row), ctrl), Some(action));
            assert_eq!(kb.triggered(&Input::key(pad), ctrl), Some(action));
            assert_eq!(kb.triggered(&Input::key(row), Mods::default()), None);
        }
        kb.set(Action::LensZoomIn, 0, None);
        assert_eq!(kb.sanitized().get(Action::LensZoomIn)[0], None);
    }

    /// World › Environment's shortcuts (menu_viewer.xml): Ctrl+Shift+U / Y /
    /// N / X / J, added to older settings files and rebindable.
    #[test]
    fn environment_shortcuts_pick_the_legacy_times_of_day() {
        let mut kb = KeyBindings::us_places([KeyCode::KeyW, KeyCode::KeyS, KeyCode::KeyA, KeyCode::KeyD]);
        kb.map.retain(|(a, _)| a.time_of_day().is_none());
        let mut kb = kb.sanitized();
        let ctrl_shift = Mods {
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        for (code, action, time) in [
            (KeyCode::KeyU, Action::EnvSunrise, 1),
            (KeyCode::KeyY, Action::EnvMidday, 2),
            (KeyCode::KeyN, Action::EnvSunset, 3),
            (KeyCode::KeyX, Action::EnvMidnight, 4),
            (KeyCode::KeyJ, Action::EnvShared, 0),
        ] {
            assert_eq!(kb.triggered(&Input::key(code), ctrl_shift), Some(action), "{code:?}");
            assert_eq!(kb.users(&kb.get(action)[0].clone().expect("default binding")), [action]);
            assert_eq!(action.time_of_day(), Some(time));
        }
        assert_eq!(Action::Fly.time_of_day(), None);
        kb.set(Action::EnvMidnight, 0, None);
        assert_eq!(kb.sanitized().get(Action::EnvMidnight)[0], None);
    }

    #[test]
    fn defaults_cover_every_action() {
        let kb = KeyBindings::default();
        for a in all_actions() {
            assert!(kb.map.iter().any(|(x, _)| *x == a), "{a:?}");
        }
    }

    #[test]
    fn ik_shortcut_is_added_to_existing_settings_and_can_be_rebound_or_cleared() {
        let mut keys = KeyBindings::default();
        keys.map.retain(|(action, _)| *action != Action::IkEdit);
        let mut keys = keys.sanitized();
        let ctrl = Mods {
            ctrl: true,
            ..Default::default()
        };
        assert_eq!(keys.triggered(&Input::key(KeyCode::KeyK), ctrl), Some(Action::IkEdit));
        assert_eq!(keys.triggered(&Input::key(KeyCode::KeyK), Mods::default()), None);
        keys.set(Action::IkEdit, 0, kmod(KeyCode::KeyJ, true, false));
        let mut keys = keys.sanitized();
        assert_eq!(keys.triggered(&Input::key(KeyCode::KeyK), ctrl), None);
        assert_eq!(keys.triggered(&Input::key(KeyCode::KeyJ), ctrl), Some(Action::IkEdit));
        keys.set(Action::IkEdit, 0, None);
        assert_eq!(keys.sanitized().triggered(&Input::key(KeyCode::KeyJ), ctrl), None);
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
        let mods = |ctrl, shift, alt| Mods { ctrl, shift, alt };
        for (layout, m_key) in [
            (layout::KeyboardLayout::wasd(), KeyCode::KeyM),
            (layout::KeyboardLayout::zqsd(), KeyCode::Semicolon),
            (layout::KeyboardLayout::qwertz(), KeyCode::KeyM),
        ] {
            let kb = KeyBindings::for_layout(&layout);
            let m = Input::key(m_key);
            assert_eq!(kb.triggered(&m, Mods::default()), Some(Action::Mouselook));
            assert_eq!(kb.triggered(&m, mods(true, true, false)), Some(Action::Minimap));
            assert_eq!(kb.triggered(&m, mods(true, false, false)), Some(Action::WorldMap));
            assert_eq!(kb.triggered(&m, mods(false, true, true)), None);
        }
    }

    /// Firestorm binds a letter shortcut to the key Windows names that way
    /// in the active layout (LLKeyboardWin32): the M of AZERTY is right of L,
    /// its ` (VK_OEM_3) is ù, and QWERTZ's ` is ö.
    #[test]
    fn letter_shortcuts_follow_the_layout_and_movement_keeps_its_places() {
        use KeyCode as K;
        let us = layout::KeyboardLayout::wasd();
        let fr = layout::KeyboardLayout::zqsd();
        let be = layout::KeyboardLayout::belgian();
        let de = layout::KeyboardLayout::qwertz();
        // action, its modifiers, then its key on US, French, Belgian, German
        let expected = [
            (
                Action::Mouselook,
                (false, false, false),
                [K::KeyM, K::Semicolon, K::Semicolon, K::KeyM],
            ),
            (
                Action::WorldMap,
                (true, false, false),
                [K::KeyM, K::Semicolon, K::Semicolon, K::KeyM],
            ),
            (Action::Minimap, (true, true, false), [K::KeyM, K::Semicolon, K::Semicolon, K::KeyM]),
            (Action::People, (true, true, false), [K::KeyA, K::KeyQ, K::KeyQ, K::KeyA]),
            (
                Action::PhotoToDisk,
                (true, false, false),
                [K::Backquote, K::Quote, K::Quote, K::Semicolon],
            ),
            (Action::Fly, (false, false, false), [K::KeyF; 4]),
            (Action::CameraControls, (true, true, false), [K::KeyC; 4]),
            (Action::AlwaysRun, (true, false, false), [K::KeyR; 4]),
            (Action::NearbyChat, (true, false, false), [K::KeyH; 4]),
            (Action::Conversations, (true, false, false), [K::KeyT; 4]),
            (Action::Inventory, (true, false, false), [K::KeyI; 4]),
            (Action::IkEdit, (true, false, false), [K::KeyK; 4]),
            (Action::Gestures, (true, false, false), [K::KeyG; 4]),
            (Action::TeleportHistory, (false, false, true), [K::KeyH; 4]),
            (Action::TeleportHome, (true, true, false), [K::KeyH; 4]),
            (Action::Search, (true, false, false), [K::KeyF; 4]),
            (Action::Performance, (true, true, false), [K::Digit1; 4]),
            (Action::Preferences, (true, false, false), [K::KeyP; 4]),
            (Action::Import, (true, false, false), [K::KeyU; 4]),
            (Action::ShowTransparency, (true, false, true), [K::KeyT; 4]),
            (Action::Beacons, (true, true, true), [K::KeyN; 4]),
            (Action::PropertyLines, (true, true, true), [K::KeyP; 4]),
            (Action::LandOwners, (true, false, true), [K::KeyO; 4]),
            (
                Action::HideParticles,
                (true, true, true),
                [K::Equal, K::Equal, K::Slash, K::BracketRight],
            ),
            (Action::DevToolbox, (true, false, true), [K::KeyD; 4]),
            (Action::Rebake, (true, false, true), [K::KeyR; 4]),
            (Action::StopAnimations, (false, true, true), [K::KeyA, K::KeyQ, K::KeyQ, K::KeyA]),
            (Action::ResyncAnimations, (true, false, false), [K::KeyS; 4]),
            (Action::RefreshAttachments, (false, true, true), [K::KeyR; 4]),
            (Action::PhotoWindow, (true, true, false), [K::KeyS; 4]),
            (Action::SitGround, (false, true, true), [K::KeyS; 4]),
        ];
        for (i, layout) in [&us, &fr, &be, &de].into_iter().enumerate() {
            let kb = KeyBindings::for_layout(layout);
            for (action, (ctrl, shift, alt), keys) in expected {
                let binding = Binding {
                    input: Input::key(keys[i]),
                    ctrl,
                    shift,
                    alt,
                };
                assert!(kb.get(action).contains(&Some(binding)), "{action:?} on layout {i}");
            }
            // movement keeps the places under the hand: WASD / ZQSD, E and C
            for (action, key) in [
                (Action::Forward, K::KeyW),
                (Action::Back, K::KeyS),
                (Action::Left, K::KeyA),
                (Action::Right, K::KeyD),
                (Action::Up, K::KeyE),
                (Action::Down, K::KeyC),
            ] {
                assert_eq!(kb.get(action)[1], k(key), "{action:?} on layout {i}");
            }
            // and no default steals another's key
            for (action, binds) in &kb.map {
                for b in binds.iter().flatten() {
                    assert_eq!(kb.users(b), vec![*action], "{b:?} on layout {i}");
                }
            }
        }
    }

    #[test]
    fn former_letter_defaults_move_once_and_choices_stay() {
        use KeyCode as K;
        let fr = layout::KeyboardLayout::zqsd();
        // a profile saved on AZERTY before the fix: letters at their US places
        let mut kb = KeyBindings::us_places(fr.movement_keys());
        kb.set(Action::People, 0, kmod(K::KeyK, true, true)); // the user's own key
        kb.set(Action::WorldMap, 0, None); // cleared by the user
        kb.set(Action::Inventory, 1, kmod(K::Quote, true, false)); // ù already taken
        kb.follow_letters_of(&fr);
        assert_eq!(kb.get(Action::Mouselook)[0], k(K::Semicolon));
        assert_eq!(kb.get(Action::Minimap)[0], kmod(K::Semicolon, true, true));
        assert_eq!(kb.get(Action::People)[0], kmod(K::KeyK, true, true));
        assert_eq!(kb.get(Action::WorldMap), [None, None]);
        assert_eq!(kb.get(Action::PhotoToDisk)[0], kmod(K::Backquote, true, false));
        assert_eq!(kb.get(Action::Forward)[1], k(K::KeyW));
        let migrated = kb.clone();
        kb.follow_letters_of(&fr);
        assert_eq!(kb, migrated);

        // a QWERTY profile has nothing to move
        let us = layout::KeyboardLayout::wasd();
        let mut kb = KeyBindings::us_places(us.movement_keys());
        kb.follow_letters_of(&us);
        assert_eq!(kb, KeyBindings::for_layout(&us));
        // and an AZERTY profile with untouched defaults ends on the new ones
        let mut kb = KeyBindings::us_places(fr.movement_keys());
        kb.follow_letters_of(&fr);
        assert_eq!(kb, KeyBindings::for_layout(&fr));
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
        kb.map.retain(|(a, _)| *a != Action::ResyncAnimations);
        kb.map.push((Action::Forward, [None, None])); // duplicate
        let kb = kb.sanitized();
        assert_eq!(kb.map.iter().filter(|(a, _)| *a == Action::Forward).count(), 1);
        assert!(kb.map.iter().any(|(a, _)| *a == Action::ToggleMic));
        assert_eq!(
            kb.get(Action::ResyncAnimations),
            KeyBindings::default().get(Action::ResyncAnimations)
        );
    }

    #[test]
    fn resync_shortcut_does_not_walk_back_or_take_snapshot_and_sit_keys() {
        let kb = KeyBindings::default();
        let input = Input::key(KeyCode::KeyS);
        let mods = Mods {
            ctrl: true,
            ..Default::default()
        };
        let mut down = HashSet::from([input.clone()]);
        assert_eq!(kb.update_held_input(&mut down, &input, true, mods), Some(Action::ResyncAnimations));
        assert!(down.is_empty());
        assert!(!kb.held(Action::Back, &down, Mods::default()));
        assert_eq!(kb.triggered(&input, Mods { shift: true, ..mods }), Some(Action::PhotoWindow));
        assert_eq!(
            kb.triggered(
                &input,
                Mods {
                    alt: true,
                    shift: true,
                    ctrl: false
                }
            ),
            Some(Action::SitGround)
        );
    }

    #[test]
    fn fresh_defaults_bind_movement_in_the_secondary_slots() {
        for layout in [
            layout::KeyboardLayout::wasd(),
            layout::KeyboardLayout::zqsd(),
            layout::KeyboardLayout::qwertz(),
            Default::default(),
        ] {
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

#[cfg(test)]
mod passage_tests {
    use super::*;
    #[test]
    fn passage_keycaps_keep_custom_secondary_mouse_and_unassigned_bindings() {
        let mut keys = KeyBindings::default();
        keys.set(Action::Up, 0, k(KeyCode::F10));
        keys.set(Action::Up, 1, kalt(KeyCode::F11, false, false));
        assert_eq!(keys.pass_wall_bindings(), [kmod(KeyCode::F10, true, false), None]);
        let mouse = Binding::captured(
            Input::Mouse("Other9".into()),
            Mods {
                shift: true,
                ..Default::default()
            },
        );
        keys.set(Action::PassWall, 0, None);
        keys.set(Action::PassWall, 1, Some(mouse.clone()));
        assert_eq!(keys.pass_wall_bindings(), [None, Some(mouse)]);
        keys.set(Action::PassWall, 1, None);
        keys.set(Action::Up, 0, None);
        assert_eq!(keys.pass_wall_bindings(), [None, None]);
    }

    #[test]
    fn passage_follows_jump_rebindings_without_overriding_another_shortcut() {
        let mut keys = KeyBindings::default();
        let ctrl = Mods {
            ctrl: true,
            ..Default::default()
        };
        assert!(keys.pass_wall_triggered(&Input::key(KeyCode::PageUp), ctrl));
        assert!(keys.pass_wall_triggered(&Input::key(KeyCode::KeyE), ctrl));
        assert!(!keys.pass_wall_triggered(&Input::key(KeyCode::KeyE), Mods::default()));
        keys.set(Action::Up, 0, k(KeyCode::Space));
        keys.set(Action::Up, 1, None);
        assert!(!keys.pass_wall_triggered(&Input::key(KeyCode::Space), ctrl));
        assert_eq!(keys.triggered(&Input::key(KeyCode::Space), ctrl), Some(Action::TextureLibrary));
        keys.set(Action::TextureLibrary, 0, None);
        assert!(keys.pass_wall_triggered(&Input::key(KeyCode::Space), ctrl));
        assert!(!keys.pass_wall_triggered(&Input::key(KeyCode::PageUp), ctrl));
        keys.set(Action::Inventory, 0, kmod(KeyCode::Space, true, false));
        assert!(!keys.pass_wall_triggered(&Input::key(KeyCode::Space), ctrl));
        assert_eq!(keys.triggered(&Input::key(KeyCode::Space), ctrl), Some(Action::Inventory));
    }
    #[test]
    fn passage_migrates_the_former_default_and_preserves_custom_bindings() {
        let mut keys = KeyBindings::default();
        keys.set(Action::PassWall, 0, kalt(KeyCode::KeyG, true, false));
        assert_eq!(keys.sanitized().get(Action::PassWall), [None, None]);
        let mut keys = KeyBindings::default();
        keys.set(Action::PassWall, 0, kmod(KeyCode::KeyJ, true, false));
        let keys = keys.sanitized();
        let ctrl = Mods {
            ctrl: true,
            ..Default::default()
        };
        assert!(keys.pass_wall_triggered(&Input::key(KeyCode::KeyJ), ctrl));
        assert!(!keys.pass_wall_triggered(&Input::key(KeyCode::KeyE), ctrl));
    }
}
