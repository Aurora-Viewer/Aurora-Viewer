//! Rebindable controls (Firestorm "Contrôles > Raccourcis"): each action
//! has up to two bindings, a keyboard key or a mouse button (middle, back,
//! forward, others) with optional modifiers. Keys are stored by their winit
//! physical code name so the layout does not matter (ZQSD on AZERTY = WASD).

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
            MouseButton::Middle => Some(Input::Mouse("Middle".into())),
            MouseButton::Back => Some(Input::Mouse("Back".into())),
            MouseButton::Forward => Some(Input::Mouse("Forward".into())),
            MouseButton::Other(n) => Some(Input::Mouse(format!("Other{n}"))),
        }
    }

    /// Modifier keys alone are not bindable (they qualify the binding).
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
                "Middle" => "Clic molette".into(),
                "Back" => "Souris précédent".into(),
                "Forward" => "Souris suivant".into(),
                other => format!("Souris {}", other.trim_start_matches("Other")),
            },
            Input::Key(k) => key_label(k),
        }
    }
}

/// Characters seen for physical keys (the real keyboard layout), learned
/// from key events; labels fall back to an AZERTY guess before that.
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
    if !k.starts_with("Numpad")
        && let Some(l) = learned(k)
    {
        return l;
    }
    let fixed = match k {
        "ArrowUp" => "Flèche haut",
        "ArrowDown" => "Flèche bas",
        "ArrowLeft" => "Flèche gauche",
        "ArrowRight" => "Flèche droite",
        "PageUp" => "Page préc.",
        "PageDown" => "Page suiv.",
        "Home" => "Origine",
        "End" => "Fin",
        "Insert" => "Inser",
        "Delete" => "Suppr",
        "Backspace" => "Retour arrière",
        "Enter" => "Entrée",
        "NumpadEnter" => "Entrée (pavé)",
        "Escape" => "Échap",
        "Space" => "Espace",
        "Tab" => "Tab",
        "CapsLock" => "Verr. maj",
        "Backquote" => "²",
        "Minus" => ")",
        "Equal" => "=",
        "BracketLeft" => "^",
        "BracketRight" => "$",
        "Semicolon" => "M",
        "Quote" => "ù",
        "Backslash" => "*",
        "Comma" => ";",
        "Period" => ":",
        "Slash" => "!",
        "IntlBackslash" => "<",
        "NumpadAdd" => "+ (pavé)",
        "NumpadSubtract" => "- (pavé)",
        "NumpadMultiply" => "* (pavé)",
        "NumpadDivide" => "/ (pavé)",
        "NumpadDecimal" => ". (pavé)",
        _ => "",
    };
    if !fixed.is_empty() {
        return fixed.into();
    }
    if let Some(c) = k.strip_prefix("Key") {
        // physical position on a QWERTY board: show the AZERTY letter
        return match c {
            "Q" => "A",
            "A" => "Q",
            "W" => "Z",
            "Z" => "W",
            "M" => ",",
            other => other,
        }
        .into();
    }
    if let Some(d) = k.strip_prefix("Digit") {
        return d.into();
    }
    if let Some(d) = k.strip_prefix("Numpad") {
        return format!("Pavé {d}");
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
            s.push_str("Maj+");
        }
        s.push_str(&self.input.label());
        s
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    Forward,
    Back,
    /// Turn (strafe with Shift, or always when "flèches = pas latéraux").
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
    Performance,
    Preferences,
    /// « Afficher la transparence » (Firestorm Ctrl+Alt+T).
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
    )
}

/// Actions by section, with their French labels (preferences table order).
pub const SECTIONS: &[(&str, &[(Action, &str)])] = &[
    (
        "Déplacements",
        &[
            (Action::Forward, "Avancer"),
            (Action::Back, "Reculer"),
            (Action::Left, "Tourner à gauche"),
            (Action::Right, "Tourner à droite"),
            (Action::StrafeLeft, "Pas latéral gauche"),
            (Action::StrafeRight, "Pas latéral droit"),
            (Action::Up, "Sauter / monter"),
            (Action::Down, "S'accroupir / descendre"),
            (Action::Fly, "Voler / atterrir"),
            (Action::AlwaysRun, "Toujours courir"),
        ],
    ),
    (
        "Caméra",
        &[
            (Action::Mouselook, "Vue subjective"),
            (Action::ResetCamera, "Revenir à l'avatar"),
            (Action::CamOrbitCw, "Tourner autour (sens horaire)"),
            (Action::CamOrbitCcw, "Tourner autour (sens inverse)"),
            (Action::CamOrbitOver, "Passer au-dessus"),
            (Action::CamOrbitUnder, "Passer en dessous"),
            (Action::CamZoomIn, "Rapprocher la caméra"),
            (Action::CamZoomOut, "Éloigner la caméra"),
            (Action::CamPanLeft, "Décaler la vue à gauche"),
            (Action::CamPanRight, "Décaler la vue à droite"),
            (Action::CamPanUp, "Décaler la vue en haut"),
            (Action::CamPanDown, "Décaler la vue en bas"),
        ],
    ),
    (
        "Communication",
        &[
            (Action::Chat, "Écrire dans le chat"),
            (Action::PushToTalk, "Parler (push-to-talk)"),
            (Action::ToggleMic, "Activer / couper le micro"),
        ],
    ),
    (
        "Fenêtres",
        &[
            (Action::Conversations, "Conversations"),
            (Action::People, "Personnes"),
            (Action::Inventory, "Inventaire"),
            (Action::Minimap, "Mini-carte"),
            (Action::WorldMap, "Carte du monde"),
            (Action::Performance, "Performances"),
            (Action::Preferences, "Préférences"),
        ],
    ),
    ("Débogage", &[(Action::ShowTransparency, "Afficher la transparence")]),
];

/// French label of an action.
pub fn label(a: Action) -> &'static str {
    SECTIONS
        .iter()
        .flat_map(|(_, list)| list.iter())
        .find(|(x, _)| *x == a)
        .map(|(_, l)| *l)
        .unwrap_or("?")
}

#[cfg(test)]
pub fn all_actions() -> impl Iterator<Item = Action> {
    SECTIONS.iter().flat_map(|(_, list)| list.iter().map(|(a, _)| *a))
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
        use KeyCode as K;
        let defaults: Vec<(Action, [Option<Binding>; 2])> = vec![
            (Action::Forward, [k(K::ArrowUp), None]),
            (Action::Back, [k(K::ArrowDown), None]),
            (Action::Left, [k(K::ArrowLeft), None]),
            (Action::Right, [k(K::ArrowRight), None]),
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
            (Action::PushToTalk, [Some(Binding::plain(Input::Mouse("Middle".into()))), None]),
            (Action::ToggleMic, [None, None]),
            (Action::Conversations, [kmod(K::KeyT, true, false), None]),
            (Action::People, [kmod(K::KeyA, true, true), None]),
            (Action::Inventory, [kmod(K::KeyI, true, false), None]),
            (Action::Minimap, [kmod(K::KeyM, true, true), None]),
            (Action::WorldMap, [kmod(K::KeyM, true, false), None]),
            (Action::Performance, [kmod(K::Digit1, true, true), None]),
            (Action::Preferences, [kmod(K::KeyP, true, false), None]),
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

/// Current modifier state.
#[derive(Debug, Clone, Copy, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
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

    /// Walking uses the arrow keys only (former WASD / ZQSD defaults removed
    /// from older settings files; custom bindings are kept).
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
            down.contains(&b.input)
                && fits(b)
                && !self.map.iter().any(|(x, bs)| {
                    *x != a && is_held(*x) && bs.iter().flatten().any(|o| o.input == b.input && fits(o) && weight(o) > weight(b))
                })
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
        assert_eq!(Input::key(KeyCode::KeyW).label(), "Z");
    }
}
