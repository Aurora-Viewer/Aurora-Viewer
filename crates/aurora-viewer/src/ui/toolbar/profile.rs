//! Portable toolbar presentation and its Aurora/Legacy base; no live viewer data.
//! Versioned notecard text is fully validated before replacing a toolbar.
use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use super::{Bar, BarStyle, ToolbarSettings, drawers, structure};
use crate::ui::skin::{Layout, ToolButton};

const FORMAT: &str = "aurora-toolbar-interface";
const MAX_BUTTONS: usize = 512;
const MAX_MAP: usize = 1024;
const FIELDS: [&str; 22] = [
    "buttons",
    "locked",
    "vr_button_migrated",
    "director_button_migrated",
    "import_bar_filled",
    "import_shortcut_given",
    "left",
    "right",
    "bottom_style",
    "left_style",
    "right_style",
    "top",
    "top_style",
    "block_styles",
    "appearance",
    "separators",
    "drawers",
    "shortcuts",
    "menu_layout",
    "address_visible",
    "composed",
    "chat_composed",
];

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    format: String,
    version: u8,
    base: Base,
    toolbar: ToolbarSettings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Base {
    Aurora,
    Legacy,
}

impl Base {
    pub(crate) fn from_islands(islands: bool) -> Self {
        if islands { Self::Aurora } else { Self::Legacy }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Profile {
    pub base: Base,
    pub toolbar: ToolbarSettings,
}

fn invalid() -> String {
    aurora_i18n::tr!("toolbar-notecard-invalid").into()
}

fn id(text: &str) -> bool {
    !text.is_empty() && text.len() <= 512 && text.bytes().all(|byte| byte.is_ascii_alphanumeric() || b":_-./".contains(&byte))
}

fn icon(text: &str) -> bool {
    text.len() <= 128 && text.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
}

fn caption(text: &str) -> bool {
    text.len() <= 1024 && text.chars().count() <= 256 && !text.chars().any(char::is_control)
}

fn known_fields(value: &serde_json::Value, allowed: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|map| map.keys().all(|key| allowed.contains(&key.as_str())))
}

fn fields(value: &serde_json::Value) -> bool {
    if !known_fields(value, &FIELDS) {
        return false;
    }
    let buttons = |list: &serde_json::Value| {
        list.as_array()
            .is_some_and(|items| items.iter().all(|button| known_fields(button, &["command", "icon", "tooltip"])))
    };
    for key in ["buttons", "top", "left", "right"] {
        if let Some(value) = value.get(key)
            && !value.is_null()
            && !buttons(value)
        {
            return false;
        }
    }
    for key in ["bottom_style", "top_style", "left_style", "right_style"] {
        if let Some(value) = value.get(key)
            && !known_fields(value, &["mode", "align", "sizing"])
        {
            return false;
        }
    }
    for (key, allowed) in [
        ("block_styles", &["mode", "align", "sizing"][..]),
        ("appearance", &["mode", "icon"][..]),
        ("separators", &["size", "stretch"][..]),
        ("shortcuts", &["input", "ctrl", "shift", "alt"][..]),
    ] {
        if let Some(value) = value.get(key)
            && !value
                .as_object()
                .is_some_and(|map| map.values().all(|entry| known_fields(entry, allowed)))
        {
            return false;
        }
    }
    if let Some(value) = value.get("drawers")
        && !value.as_object().is_some_and(|map| {
            map.values().all(|drawer| {
                known_fields(drawer, &["buttons", "mode", "closes", "floating"])
                    && drawer.get("buttons").is_none_or(buttons)
                    && drawer
                        .get("floating")
                        .is_none_or(|floating| known_fields(floating, &["name", "icon", "vertical", "shown"]))
            })
        })
    {
        return false;
    }
    true
}

/// Keyboard shortcuts: a key the capture would take (`shortcuts::refusal`),
/// one target each.
fn shortcuts_valid(toolbar: &ToolbarSettings) -> bool {
    let mut seen = HashSet::new();
    toolbar.shortcuts.iter().all(|(key, binding)| {
        id(key)
            && matches!(&binding.input, crate::keybinds::Input::Key(code) if code.len() <= 64 && icon(code))
            && super::shortcuts::refusal(binding).is_none()
            && seen.insert(binding)
    })
}

fn valid(toolbar: &ToolbarSettings) -> bool {
    if !toolbar.composed
        || !toolbar.chat_composed
        || toolbar.buttons.is_none()
        || toolbar.drawers.len() > 64
        || [
            toolbar.block_styles.len(),
            toolbar.appearance.len(),
            toolbar.separators.len(),
            toolbar.shortcuts.len(),
            toolbar.menu_layout.len(),
        ]
        .into_iter()
        .any(|count| count > MAX_MAP)
    {
        return false;
    }
    let mut placed = HashSet::new();
    let button = |button: &ToolButton| id(&button.command) && icon(&button.icon) && caption(&button.tooltip);
    for entry in toolbar
        .buttons
        .iter()
        .flatten()
        .chain(&toolbar.top)
        .chain(&toolbar.left)
        .chain(&toolbar.right)
    {
        if !button(entry) || !placed.insert(entry.command.as_str()) {
            return false;
        }
    }
    let roots = placed.clone();
    for (key, drawer) in &toolbar.drawers {
        // a floating block needs no button placed
        let floating = drawer.floating.as_ref();
        if !id(key)
            || !drawers::is_drawer(key)
            || (floating.is_none() && !roots.contains(key.as_str()))
            || floating.is_some_and(|floating| !caption(&floating.name) || !(floating.icon.is_empty() || icon(&floating.icon)))
        {
            return false;
        }
        for entry in &drawer.buttons {
            if !button(entry)
                || drawers::is_drawer(&entry.command)
                || entry.command == drawers::FLOATING
                || (floating.is_some() && structure::is_block(&entry.command))
                || !placed.insert(entry.command.as_str())
            {
                return false;
            }
        }
    }
    if placed.len() > MAX_BUTTONS
        || roots
            .iter()
            .any(|command| drawers::is_drawer(command) && !toolbar.drawers.contains_key(*command))
        || !toolbar.block_styles.keys().all(|key| id(key))
        || !shortcuts_valid(toolbar)
        || !toolbar
            .appearance
            .iter()
            .all(|(key, value)| id(key) && value.icon.as_deref().is_none_or(icon))
        || !toolbar.separators.iter().all(|(key, value)| {
            id(key)
                && key.starts_with(structure::PREFIX)
                && value.size.is_finite()
                && (structure::SeparatorStyle::MIN..=structure::SeparatorStyle::MAX).contains(&value.size)
        })
    {
        return false;
    }
    let mut parents = BTreeMap::new();
    let mut total = 0;
    for (parent, children) in &toolbar.menu_layout {
        if !parent.is_empty() && !id(parent) {
            return false;
        }
        total += children.len();
        if total > MAX_MAP {
            return false;
        }
        for child in children {
            if !id(child) || child == parent || parents.insert(child.as_str(), parent.as_str()).is_some() {
                return false;
            }
        }
    }
    for start in parents.keys() {
        let mut chain = HashSet::new();
        let mut current = *start;
        while !current.is_empty() {
            if !chain.insert(current) || chain.len() > 32 {
                return false;
            }
            current = parents.get(current).copied().unwrap_or_else(|| {
                if current.starts_with("menu:") {
                    current.rsplit_once('/').map_or("", |(parent, _)| parent)
                } else {
                    ""
                }
            });
        }
    }
    true
}

/// Materialize the source skin and bar defaults, retaining the bars' base.
pub(crate) fn snapshot(settings: &ToolbarSettings, layout: &Layout, islands: bool) -> Profile {
    let mut toolbar = settings.clone();
    toolbar.compose_top();
    toolbar.compose_chat(layout);
    toolbar.buttons = Some(toolbar.buttons(layout).to_vec());
    toolbar.sanitize();
    // This is a complete snapshot: discovery must not append a removed control.
    toolbar.vr_button_migrated = true;
    toolbar.director_button_migrated = true;
    toolbar.import_bar_filled = true;
    toolbar.import_shortcut_given = true;
    for (bar, style) in [
        (Bar::Bottom, &mut toolbar.bottom_style),
        (Bar::Left, &mut toolbar.left_style),
        (Bar::Right, &mut toolbar.right_style),
        (Bar::Top, &mut toolbar.top_style),
    ] {
        let resolved = style.resolve(bar, islands);
        *style = BarStyle {
            mode: Some(resolved.mode),
            align: Some(resolved.align),
            sizing: Some(resolved.sizing),
        };
    }
    Profile {
        base: Base::from_islands(islands),
        toolbar,
    }
}

pub(crate) fn write(settings: &ToolbarSettings, layout: &Layout, islands: bool) -> Result<String, String> {
    let profile = snapshot(settings, layout, islands);
    let text = serde_json::to_string_pretty(&Document {
        format: FORMAT.into(),
        version: 2,
        base: profile.base,
        toolbar: profile.toolbar,
    })
    .map_err(|_| invalid())?;
    if text.len() > aurora_net::inventory::notecard::MAX_TEXT_BYTES {
        return Err(aurora_i18n::tr!("toolbar-notecard-too-large").into());
    }
    read(&text)?;
    Ok(text)
}

pub(crate) fn read(text: &str) -> Result<Profile, String> {
    if text.len() > aurora_net::inventory::notecard::MAX_TEXT_BYTES {
        return Err(invalid());
    }
    let value: serde_json::Value = serde_json::from_str(text).map_err(|_| invalid())?;
    if !value.get("toolbar").is_some_and(fields) {
        return Err(invalid());
    }
    let mut document: Document = serde_json::from_value(value).map_err(|_| invalid())?;
    if document.format != FORMAT || document.version != 2 || !valid(&document.toolbar) {
        return Err(invalid());
    }
    document.toolbar.vr_button_migrated = true;
    document.toolbar.director_button_migrated = true;
    document.toolbar.import_bar_filled = true;
    document.toolbar.import_shortcut_given = true;
    Ok(Profile {
        base: document.base,
        toolbar: document.toolbar,
    })
}

/// Restore the bars' base without changing unrelated window or graphics styles.
pub(crate) fn apply(profile: Profile, settings: &mut crate::settings::Settings) {
    settings.classic_bars = profile.base == Base::Legacy;
    settings.show_bug_report = profile.toolbar.placed("report_bug", &Layout::default());
    let mut toolbar = profile.toolbar;
    toolbar.migrate_build_icons();
    settings.toolbar = Some(toolbar);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::toolbar::{ButtonMode, appearance::Appearance, presets::Preset};

    #[test]
    fn interface_notecard_roundtrip_preserves_all_bar_controls_and_materializes_defaults() {
        let layout = Layout::default();
        for (preset, islands) in Preset::ALL
            .into_iter()
            .flat_map(|preset| [true, false].map(|islands| (preset, islands)))
        {
            let mut toolbar = preset.layout(&layout);
            toolbar.appearance.insert(
                "inventory".into(),
                Appearance {
                    mode: Some(ButtonMode::TextOnly),
                    icon: Some("star-filled".into()),
                },
            );
            toolbar.appearance.insert(
                "menu-entry:menu:common-me".into(),
                Appearance {
                    mode: None,
                    icon: Some("user-filled".into()),
                },
            );
            let drawer = super::super::drawers::create(&mut toolbar, Bar::Right, &layout);
            // the import bar and a floating block of the user's, its button
            // placed nowhere, keep their buttons and their options
            // the Aurora layouts bring it (`assets/toolbar/aurora.json`)
            toolbar.ensure_import_bar(&layout);
            assert!(!toolbar.drawers[super::super::drawers::IMPORT_BAR].buttons.is_empty());
            let floating = super::super::drawers::create_floating(&mut toolbar, "Tools");
            toolbar
                .buttons
                .as_mut()
                .expect("explicit bottom")
                .retain(|button| button.command != "soundexplorer");
            let block = toolbar.drawers.get_mut(&floating).expect("floating block");
            block.buttons = vec![ToolButton {
                command: "soundexplorer".into(),
                icon: "volume".into(),
                tooltip: String::new(),
            }];
            block.floating.as_mut().expect("floating").vertical = true;
            let shortcut = |code: &str, ctrl: bool| crate::keybinds::Binding {
                input: crate::keybinds::Input::Key(code.into()),
                ctrl,
                shift: false,
                alt: !ctrl,
            };
            toolbar.shortcuts.insert("soundexplorer".into(), shortcut("KeyK", true));
            toolbar.shortcuts.insert(floating.clone(), shortcut("KeyJ", false));
            toolbar
                .buttons
                .as_mut()
                .expect("explicit bottom")
                .retain(|button| button.command != "inventory");
            toolbar.drawers.get_mut(&drawer).expect("drawer").buttons = vec![ToolButton {
                command: "inventory".into(),
                icon: "Command_Inventory_Icon".into(),
                tooltip: String::new(),
            }];
            toolbar
                .menu_layout
                .insert("menu:common-world".into(), vec!["menu:common-help".into()]);
            let gap = toolbar
                .top
                .iter()
                .find(|button| structure::is_separator(&button.command))
                .map(|button| button.command.clone())
                .unwrap_or_else(|| {
                    let gap = structure::separator("profile-roundtrip-gap");
                    let id = gap.command.clone();
                    toolbar.top.insert(1, gap);
                    id
                });
            toolbar
                .separators
                .insert(gap, structure::SeparatorStyle { size: 48.0, stretch: true });
            let text = write(&toolbar, &layout, islands).expect("portable interface");
            let profile = read(&text).expect("validated interface");
            assert_eq!(profile.base, Base::from_islands(islands));
            let mut loaded = profile.toolbar;
            assert!(!loaded.migrate_vr_button(), "a snapshot retains deliberately absent controls");
            assert_eq!(loaded.appearance, toolbar.appearance);
            assert_eq!(loaded.drawers, toolbar.drawers);
            assert_eq!(loaded.menu_layout, toolbar.menu_layout);
            assert_eq!(loaded.separators, toolbar.separators);
            assert_eq!(loaded.shortcuts, toolbar.shortcuts);
            for bar in Bar::ALL {
                assert_eq!(loaded.style(bar).resolve(bar, !islands), toolbar.style(bar).resolve(bar, islands));
                assert_eq!(loaded.bar(bar, &layout), toolbar.bar(bar, &layout));
            }
            assert_eq!(write(&loaded, &layout, islands).expect("portable again"), text);
            assert!(!text.contains("password") && !text.contains("language") && !text.contains("draw_distance"));
        }
    }

    #[test]
    fn loading_an_interface_restores_either_base_without_changing_unrelated_preferences() {
        for islands in [true, false] {
            let mut settings = crate::settings::Settings {
                classic_bars: islands,
                language: "ja".into(),
                draw_distance: 222.0,
                ..Default::default()
            };
            let mut before = serde_json::to_value(&settings).expect("settings");
            let layout = Preset::Photographer.layout(&Layout::default());
            let text = write(&layout, &Layout::default(), islands).expect("saved interface");
            let profile = read(&text).expect("saved base");
            let toolbar = profile.toolbar.clone();
            apply(profile, &mut settings);
            assert_eq!(settings.classic_bars, !islands);
            assert_eq!(settings.toolbar, Some(toolbar));
            assert!(settings.show_bug_report);
            let mut after = serde_json::to_value(&settings).expect("settings");
            for value in [&mut before, &mut after] {
                let object = value.as_object_mut().expect("settings object");
                object.remove("toolbar_common");
                object.remove("show_bug_report");
                object.remove("classic_bars");
            }
            assert_eq!(before, after);
        }
    }

    #[test]
    fn interface_notecard_rejects_oversize_wrong_versions_duplicate_controls_and_cycles() {
        let layout = Layout::default();
        let toolbar = Preset::OldSchool.layout(&layout);
        let text = write(&toolbar, &layout, true).expect("interface");
        assert!(read(&"x".repeat(aurora_net::inventory::notecard::MAX_TEXT_BYTES + 1)).is_err());
        for changed in [
            text.replace(FORMAT, "other"),
            text.replace("\"version\": 2", "\"version\": 1"),
            text.replace("\"version\": 2", "\"version\": 3"),
            text.replace("\"base\": \"aurora\"", "\"base\": \"unknown\""),
            text.replace("  \"base\": \"aurora\",\n", ""),
            text.replace("\"locked\": false", "\"locked\": false,\"credentials\":\"rejected\""),
        ] {
            assert!(read(&changed).is_err());
        }
        let mut value: serde_json::Value = serde_json::from_str(&text).expect("json");
        let first = value["toolbar"]["top"][0].clone();
        value["toolbar"]["top"].as_array_mut().expect("buttons").push(first);
        assert!(read(&value.to_string()).is_err());
        value = serde_json::from_str(&text).expect("json");
        value["toolbar"]["menu_layout"] = serde_json::json!({
            "menu:common-me": ["menu:common-world"],
            "menu:common-world": ["menu:common-me"]
        });
        assert!(read(&value.to_string()).is_err());
        value["toolbar"]["menu_layout"] = serde_json::json!({
            "menu:common-world/common-environment": ["menu:common-world"]
        });
        assert!(read(&value.to_string()).is_err());
        // a key the capture refuses, or two targets on one key
        value = serde_json::from_str(&text).expect("json");
        let copy = serde_json::json!({"input": {"Key": "KeyC"}, "ctrl": true, "shift": false, "alt": false});
        value["toolbar"]["shortcuts"] = serde_json::json!({ "inventory": copy });
        assert!(read(&value.to_string()).is_err());
        let k = serde_json::json!({"input": {"Key": "KeyK"}, "ctrl": true, "shift": false, "alt": false});
        value["toolbar"]["shortcuts"] = serde_json::json!({ "inventory": k.clone(), "people": k.clone() });
        assert!(read(&value.to_string()).is_err());
        value["toolbar"]["shortcuts"] = serde_json::json!({ "inventory": k });
        assert!(read(&value.to_string()).is_ok());
    }

    #[test]
    fn interface_notecard_uses_indented_technical_json_with_an_explicit_base() {
        let layout = Layout::default();
        let toolbar = Preset::OldSchool.layout(&layout);
        let text = write(&toolbar, &layout, true).expect("readable interface");
        assert!(text.starts_with("{\n  \"format\": "));
        assert!(text.contains("\n    \"top\": [\n      {\n"));
        assert!(text.contains("\n  \"base\": \"aurora\",\n"));
        let restored = read(&text).expect("indented configuration");
        let value: serde_json::Value = serde_json::from_str(&text).expect("json");
        assert_eq!(value.as_object().expect("document").len(), 4);
        assert_eq!(read(&value.to_string()).expect("same schema without indentation"), restored);
    }
}
