//! The settings by name, for the Debug Settings window (`ui/debug_settings.rs`):
//! the role of LLControlGroup (indra/llxml/llcontrol.cpp, originally LGPL 2.1)
//! for typed settings.
//!
//! The list is read from the settings' own serialised form, so a new setting
//! appears here without a line of code and a removed one cannot stay. A key is
//! the path of a value (`camera.fov`), the same path Preferences' rows name
//! (`ui/options/cx.rs`). A change goes back through the settings' own
//! deserialisation and `Settings::sanitized`, so it has the type and the
//! bounds of a settings file.
//!
//! Deliberate differences from the Legacy Viewer, all stricter:
//! - the login, this PC's window and folders, and the service keys are never
//!   listed (the Legacy Viewer lists what it does not store elsewhere; it
//!   hides what is marked HideFromEditor);
//! - a text value holding a UUID is never listed, whatever the setting: it may
//!   be someone's asset (AGENTS.md, "Creators' permissions");
//! - while RLVa holds a restriction, the RLVa settings cannot be changed.
//!
//! Lists, per-account maps and values with no default are not listed: they
//! have no single value to edit.

use super::Settings;
use serde_json::{Map, Value};

/// Settings of the login and of this PC, or that another path has to change
/// (the cache folder moves its files, the version drives the migrations).
const HIDDEN: &[&str] = &[
    "settings_version",
    "cache_location",
    "chat_log_location",
    "window_fullscreen",
    "new_profile",
];

/// A vector shown as one row has at most this many numbers (LLQuaternion,
/// LLColor4, LLRect in the Legacy Viewer).
const MAX_VECTOR: usize = 4;

/// The type of a value, which chooses its editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Int,
    Float,
    Text,
    /// Numbers side by side; whole numbers when `whole`.
    Vector {
        len: usize,
        whole: bool,
    },
}

impl Kind {
    fn of(value: &Value) -> Option<Kind> {
        match value {
            Value::Bool(_) => Some(Kind::Bool),
            Value::Number(n) if n.is_f64() => Some(Kind::Float),
            Value::Number(_) => Some(Kind::Int),
            Value::String(_) => Some(Kind::Text),
            Value::Array(items) if (2..=MAX_VECTOR).contains(&items.len()) && items.iter().all(Value::is_number) => Some(Kind::Vector {
                len: items.len(),
                whole: items.iter().all(|n| n.is_i64() || n.is_u64()),
            }),
            _ => None,
        }
    }

    /// Whether a value of `other` can be edited as this one: a vector's
    /// numbers may be whole in one and not in the other.
    fn accepts(self, other: Kind) -> bool {
        match (self, other) {
            (Kind::Vector { len: a, .. }, Kind::Vector { len: b, .. }) => a == b,
            _ => self == other,
        }
    }
}

/// One setting.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub key: String,
    pub kind: Kind,
    pub value: Value,
    pub default: Value,
}

impl Entry {
    /// `isDefault()`, the star of the Legacy Viewer's list.
    pub fn changed(&self) -> bool {
        self.value != self.default
    }
}

/// What RLVa forbids changing at the moment; the same rule for Preferences
/// (`ui/options/cx.rs`) and the Debug Settings window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Locks {
    /// `@setcam` and friends: LLFloaterPreference greys the camera preset.
    pub camera_preset: bool,
    /// A camera owner holds the field of view.
    pub camera_fov: bool,
    /// `@sendim`: rlvhandler.cpp hides DoNotDisturbModeResponse.
    pub sendim: bool,
    /// Any restriction is held: stricter than the Legacy Viewer, where RLVa's
    /// own settings only apply at the next start.
    pub restricted: bool,
}

impl Locks {
    pub fn locked(&self, key: &str) -> bool {
        match key {
            "camera.preset" | "camera.camera_offset" | "camera.focus_offset" | "camera.offset_scale" => self.camera_preset,
            "camera.fov" => self.camera_fov,
            "autoresponse.dnd_text" => self.sendim,
            "rlva_enabled" => self.restricted,
            _ => self.restricted && key.starts_with("rlva."),
        }
    }
}

/// Whether the setting is kept out of the list, whatever its value.
fn hidden(key: &str) -> bool {
    HIDDEN.contains(&key) || super::backup::is_this_pc(key)
}

/// A UUID anywhere in `text`: 8-4-4-4-12 hexadecimal digits.
fn holds_uuid(text: &str) -> bool {
    const GROUPS: [usize; 5] = [8, 4, 4, 4, 12];
    let bytes = text.as_bytes();
    (0..bytes.len().saturating_sub(35)).any(|start| {
        let mut at = start;
        GROUPS.iter().enumerate().all(|(i, len)| {
            let digits = bytes[at..at + len].iter().all(u8::is_ascii_hexdigit);
            at += len;
            let dash = i == GROUPS.len() - 1 || bytes[at] == b'-';
            at += 1;
            digits && dash
        })
    })
}

fn shows_an_id(value: &Value) -> bool {
    value.as_str().is_some_and(holds_uuid)
}

/// The default settings in the form the list compares with.
pub fn defaults() -> Value {
    document(&Settings::default())
}

/// The file's form of `settings`; an object even if serialisation failed, in
/// which case nothing is listed.
fn document(settings: &Settings) -> Value {
    serde_json::to_value(settings).unwrap_or_else(|e| {
        log::warn!("debug settings: the settings cannot be read ({e})");
        Value::Object(Map::new())
    })
}

/// Every setting that can be edited, sorted by key.
pub fn entries(settings: &Settings, defaults: &Value) -> Vec<Entry> {
    let mut out = Vec::new();
    let current = document(&settings.to_saved());
    if let (Value::Object(values), Value::Object(defaults)) = (&current, defaults) {
        walk("", values, defaults, &mut out);
    }
    out.sort_by(|a, b| a.key.cmp(&b.key));
    out
}

/// A setting exists when the defaults hold it: a key only some files have (an
/// account in a map, a list element) is not a setting of its own.
fn walk(prefix: &str, values: &Map<String, Value>, defaults: &Map<String, Value>, out: &mut Vec<Entry>) {
    for (name, default) in defaults {
        let key = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        let Some(value) = values.get(name) else {
            continue;
        };
        if hidden(&key) {
            continue;
        }
        if let (Value::Object(values), Value::Object(defaults)) = (value, default) {
            walk(&key, values, defaults, out);
            continue;
        }
        let (Some(kind), Some(current)) = (Kind::of(default), Kind::of(value)) else {
            continue;
        };
        if kind.accepts(current) && !shows_an_id(value) && !shows_an_id(default) {
            out.push(Entry {
                key,
                kind,
                value: value.clone(),
                default: default.clone(),
            });
        }
    }
}

/// Puts `value` in the setting `key` of the document.
fn patch(document: &mut Value, key: &str, value: Value) -> Result<(), String> {
    let mut at = document;
    for part in key.split('.') {
        at = at.get_mut(part).ok_or_else(|| format!("no setting {key}"))?;
    }
    *at = value;
    Ok(())
}

/// Sets one setting. `value` must be understood by the setting's type as a
/// settings file would be: otherwise nothing changes and the reason comes
/// back. The setting's bounds apply (`Settings::sanitized`), as for a file.
pub fn set(settings: &mut Settings, key: &str, value: Value) -> Result<(), String> {
    if hidden(key) || shows_an_id(&value) {
        return Err(format!("{key} cannot be changed here"));
    }
    let mut doc = document(settings);
    patch(&mut doc, key, value)?;
    let mut next: Settings = serde_json::from_value(doc).map_err(|e| e.to_string())?;
    next.keep_session(settings);
    *settings = next.sanitized();
    Ok(())
}

impl Settings {
    /// What the file does not hold and a change of one setting must not lose.
    fn keep_session(&mut self, current: &Settings) {
        self.debug = current.debug;
        self.hover_height_account = current.hover_height_account;
        self.photo_held = current.photo_held.clone();
        self.draw_stepping = current.draw_stepping;
        self.newer_file = current.newer_file.clone();
        self.new_profile = current.new_profile;
        self.camera_limits_restored = current.camera_limits_restored;
        self.mfa_hash = current.mfa_hash.clone();
        self.build.select_probes = current.build.select_probes;
        self.radar.alert_sounds = current.radar.alert_sounds;
        self.world_overlays.land_owners = current.world_overlays.land_owners;
        self.world_overlays.hide_particles = current.world_overlays.hide_particles;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A value different from `entry`'s, of its kind.
    fn other_value(entry: &Entry) -> Value {
        match entry.kind {
            Kind::Bool => json!(!entry.value.as_bool().expect("a bool")),
            Kind::Int => match entry.value.as_i64().expect("an integer") {
                0 => json!(1),
                n => json!(n - 1),
            },
            Kind::Float => json!(entry.value.as_f64().expect("a number") * 0.5 + 0.125),
            Kind::Text | Kind::Vector { .. } => entry.value.clone(),
        }
    }

    fn entry_of(settings: &Settings, key: &str) -> Entry {
        entries(settings, &defaults())
            .into_iter()
            .find(|e| e.key == key)
            .unwrap_or_else(|| panic!("{key} is listed"))
    }

    #[test]
    fn lists_nested_settings_sorted_with_their_kind() {
        let list = entries(&Settings::default(), &defaults());
        assert!(list.len() > 100, "{} settings", list.len());
        assert!(list.windows(2).all(|w| w[0].key < w[1].key));
        assert!(list.iter().all(|e| !e.changed()));
        assert_eq!(entry_of(&Settings::default(), "show_huds").kind, Kind::Bool);
        assert_eq!(entry_of(&Settings::default(), "camera.fov").kind, Kind::Float);
        assert_eq!(entry_of(&Settings::default(), "language").kind, Kind::Text);
    }

    #[test]
    fn the_login_this_pc_and_keys_are_never_listed() {
        let mut s = Settings {
            username: "someone".into(),
            remember_username: true,
            last_login: Some(("synthetic-grid".into(), "someone".into())),
            ..Settings::default()
        };
        s.gifs.giphy_key = "a-service-key".into();
        s.gifs.klipy_key = "another-key".into();
        let list = entries(&s, &defaults());
        for key in [
            "username",
            "remember_username",
            "last_login",
            "remember_password",
            "mfa_hash",
            "grid",
            "custom_login_uri",
            "start_location",
            "start_region",
            "window_x",
            "cache_location",
            "chat_log_location",
            "settings_version",
            "gifs.giphy_key",
            "gifs.klipy_key",
            "gifs.customer_id",
            "audio.output_device",
            "audio.input_device",
        ] {
            assert!(!list.iter().any(|e| e.key == key), "{key} is listed");
            assert!(set(&mut s, key, json!("x")).is_err() || !hidden(key), "{key} can be set");
        }
        for e in &list {
            let leaf = e.key.rsplit('.').next().expect("a key");
            for word in ["password", "token", "secret", "credential", "hash"] {
                assert!(!leaf.contains(word), "{} looks like a credential", e.key);
            }
            assert!(
                !leaf.ends_with("_key") || e.kind != Kind::Text,
                "{} looks like a service key",
                e.key
            );
        }
    }

    #[test]
    fn a_text_holding_an_id_is_neither_listed_nor_written() {
        let id = "89556747-24cb-43ed-920b-47caed15465f";
        assert!(holds_uuid(id));
        assert!(holds_uuid(&format!("secondlife:///app/agent/{id}/about")));
        assert!(!holds_uuid("not-an-id"));
        assert!(!holds_uuid("89556747-24cb-43ed-920b"));
        assert!(!holds_uuid(""));
        // multi-byte text around the place an id would be
        assert!(!holds_uuid(&"é".repeat(40)));

        let mut s = Settings::default();
        assert!(
            !entries(&s, &defaults())
                .iter()
                .any(|e| shows_an_id(&e.value) || shows_an_id(&e.default))
        );
        assert!(!entries(&s, &defaults()).iter().any(|e| e.key == "rlva.blocked_experiences"));
        assert!(set(&mut s, "language", json!(id)).is_err());
        // a setting that came to hold one leaves the list
        s.browser_home_page = format!("https://example.org/{id}");
        assert!(!entries(&s, &defaults()).iter().any(|e| e.key == "browser_home_page"));
    }

    #[test]
    fn a_change_touches_that_setting_alone() {
        let d = Settings::default();
        let before = entries(&d, &defaults());
        let mut offenders = Vec::new();
        for entry in &before {
            let mut s = d.clone();
            set(&mut s, &entry.key, other_value(entry)).unwrap_or_else(|e| panic!("{}: {e}", entry.key));
            let after = entries(&s, &defaults());
            assert_eq!(before.len(), after.len(), "{}", entry.key);
            let others: Vec<&str> = before
                .iter()
                .zip(&after)
                .filter(|(b, a)| b.key != entry.key && b.value != a.value)
                // the one setting `sanitized` makes follow another
                .filter(|(b, _)| !(entry.key == "force_midday" && b.key == "time_of_day"))
                .map(|(b, _)| b.key.as_str())
                .collect();
            if !others.is_empty() {
                offenders.push(format!("{} -> {}", entry.key, others.join(", ")));
            }
        }
        assert!(offenders.is_empty(), "{}", offenders.join("\n"));
    }

    #[test]
    fn a_value_of_the_wrong_type_changes_nothing() {
        let mut s = Settings::default();
        let before = document(&s);
        assert!(set(&mut s, "show_huds", json!("yes")).is_err());
        assert!(set(&mut s, "camera.fov", json!("wide")).is_err());
        assert!(set(&mut s, "update_mode", json!("not a mode")).is_err());
        assert!(set(&mut s, "no.such.setting", json!(1)).is_err());
        assert_eq!(before, document(&s));
    }

    #[test]
    fn a_change_is_listed_as_changed_and_reset_by_its_default() {
        let d = defaults();
        let mut s = Settings::default();
        let was = s.show_huds;
        set(&mut s, "show_huds", json!(!was)).expect("set");
        assert_eq!(s.show_huds, !was);
        let entry = entry_of(&s, "show_huds");
        assert!(entry.changed());
        set(&mut s, "show_huds", entry.default.clone()).expect("reset");
        assert_eq!(s.show_huds, was);
        assert!(!entries(&s, &d).iter().any(Entry::changed));
    }

    #[test]
    fn the_bounds_of_a_settings_file_apply() {
        let mut s = Settings::default();
        set(&mut s, "fps_limit", json!(1_000_000)).expect("set");
        assert_eq!(s.fps_limit, s.clone().sanitized().fps_limit);
        assert!(s.fps_limit < 1_000_000);
    }

    #[test]
    fn what_the_file_does_not_hold_survives_a_change() {
        let mut s = Settings {
            new_profile: true,
            camera_limits_restored: true,
            hover_height_account: Some(uuid::Uuid::from_u128(7)),
            mfa_hash: "kept".into(),
            ..Settings::default()
        };
        s.username = "someone".into();
        s.debug.wireframe = true;
        s.build.select_probes = true;
        s.radar.alert_sounds = true;
        s.world_overlays.land_owners = true;
        s.world_overlays.hide_particles = true;
        set(&mut s, "show_huds", json!(false)).expect("set");
        assert!(s.new_profile && s.camera_limits_restored && s.debug.wireframe);
        assert!(s.build.select_probes && s.radar.alert_sounds);
        assert!(s.world_overlays.land_owners && s.world_overlays.hide_particles);
        assert_eq!(s.hover_height_account, Some(uuid::Uuid::from_u128(7)));
        assert_eq!(s.mfa_hash, "kept");
        assert_eq!(s.username, "someone");
    }

    #[test]
    fn rlva_locks_name_the_settings_they_hold() {
        let free = Locks::default();
        for key in [
            "camera.preset",
            "camera.fov",
            "autoresponse.dnd_text",
            "rlva_enabled",
            "rlva.allow_ooc",
        ] {
            assert!(!free.locked(key), "{key}");
        }
        let camera = Locks {
            camera_preset: true,
            ..Locks::default()
        };
        assert!(camera.locked("camera.camera_offset") && !camera.locked("camera.fov"));
        let fov = Locks {
            camera_fov: true,
            ..Locks::default()
        };
        assert!(fov.locked("camera.fov") && !fov.locked("camera.preset"));
        let im = Locks {
            sendim: true,
            ..Locks::default()
        };
        assert!(im.locked("autoresponse.dnd_text") && !im.locked("autoresponse.autorespond_text"));
        let held = Locks {
            restricted: true,
            ..Locks::default()
        };
        assert!(held.locked("rlva_enabled") && held.locked("rlva.allow_ooc"));
        assert!(!held.locked("rlva_something_else") && !held.locked("show_huds"));
    }
}
