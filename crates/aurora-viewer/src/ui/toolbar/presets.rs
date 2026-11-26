//! User-requested layouts over the shared toolbar and interface-look settings.
//! The traditional command order follows skins/default/toolbars.xml and the
//! native upload types follow indra/newview/llviewermenufile.cpp.

use std::sync::LazyLock;

use super::{Align, BarStyle, ButtonMode, Sizing, ToolbarSettings, catalogue, structure};
use crate::{
    settings::Settings,
    ui::skin::{Layout, ToolButton},
};

// Bundled presentation data is validated by the preset and profile tests. Keep
// captions live through the catalogue, rather than freezing an export's locale.
static AURORA: LazyLock<ToolbarSettings> = LazyLock::new(|| bundled(include_str!("../../../assets/toolbar/aurora.json")));
static OLD_SCHOOL: LazyLock<ToolbarSettings> = LazyLock::new(|| bundled(include_str!("../../../assets/toolbar/aurora-old-school.json")));
static BUILDER: LazyLock<ToolbarSettings> = LazyLock::new(|| bundled(include_str!("../../../assets/toolbar/aurora-builder.json")));

fn bundled(text: &str) -> ToolbarSettings {
    super::profile::read(text)
        .expect("bundled toolbar preset must be a valid interface profile")
        .toolbar
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    Light,
    OldSchool,
    Build,
    Photographer,
    Legacy,
}

impl Preset {
    pub const ALL: [Self; 5] = [Self::Light, Self::OldSchool, Self::Build, Self::Photographer, Self::Legacy];

    pub fn label(self) -> &'static str {
        aurora_i18n::text(match self {
            Self::Light => "toolbar-preset-light",
            Self::OldSchool => "toolbar-preset-old-school",
            Self::Build => "toolbar-preset-build",
            Self::Photographer => "toolbar-preset-photographer",
            Self::Legacy => "toolbar-preset-legacy",
        })
    }

    /// Its key in `ToolbarSettings::hidden_interfaces` and in its button's
    /// command in an Interfaces block (`drawers::INTERFACES`).
    pub fn key(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::OldSchool => "old_school",
            Self::Build => "build",
            Self::Photographer => "photographer",
            Self::Legacy => "legacy",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|preset| preset.key() == key)
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Light => "sparkles-filled",
            Self::OldSchool => "layout-align-bottom-filled",
            Self::Build => "box",
            Self::Photographer => "camera-filled",
            Self::Legacy => "layout-grid-filled",
        }
    }

    pub fn layout(self, skin: &Layout) -> ToolbarSettings {
        match self {
            Self::Light => AURORA.clone(),
            Self::OldSchool => OLD_SCHOOL.clone(),
            Self::Build => BUILDER.clone(),
            Self::Photographer => {
                let mut settings = AURORA.clone();
                // A focused extension of the Legacy Viewer's Phototools:
                // camera/capture together, graphics and local lighting together.
                settings.left = PHOTO_CAMERA
                    .into_iter()
                    .filter(|id| settings.location(id, skin).is_none())
                    .filter_map(button)
                    .collect();
                let mut tools = PHOTO_SETTINGS.iter().filter_map(|id| button(id)).collect::<Vec<_>>();
                tools.push(ToolButton {
                    command: PERSONAL_LIGHTING.into(),
                    icon: "adjustments-filled".into(),
                    tooltip: "@common-personal-lighting".into(),
                });
                insert_top_block(&mut settings, tools, "preset-photographer");
                settings
            }
            Self::Legacy => {
                let mut settings = ToolbarSettings::default();
                settings.compose_top();
                settings.compose_chat(skin);
                // Unsupported historical buttons are replaced with Aurora's
                // existing equivalents, without inventing inert commands.
                let tools = [
                    "conversations",
                    "speak",
                    "voice",
                    "move",
                    "view",
                    "people",
                    "appearance",
                    "search",
                    "worldmap",
                    "minimap",
                    "snapshot",
                    "inventory",
                    "ao",
                    "preferences",
                ];
                let mut bottom = button("bar_chat").into_iter().collect::<Vec<_>>();
                bottom.push(structure::separator("preset-chat-tools"));
                bottom.extend(tools.iter().filter_map(|id| button(id)));
                settings.buttons = Some(bottom);
                settings.bottom_style = BarStyle {
                    mode: Some(ButtonMode::IconsOnly),
                    sizing: Some(Sizing::Fill),
                    align: Some(Align::Start),
                };
                settings.left = button("places").into_iter().collect();
                settings.top = [
                    "aurora_menu",
                    "report_bug",
                    "bar_navigation",
                    "bar_conversations",
                    "bar_status",
                    "bar_balance",
                    "bar_clock",
                    "bar_ping",
                    "bar_fps",
                    "bar_network",
                    "bar_media",
                    "bar_music",
                    "bar_volume",
                    "bar_notifications",
                ]
                .iter()
                .filter_map(|id| button(id))
                .collect();
                settings
            }
        }
    }

    pub fn apply(self, settings: &mut Settings, skin: &Layout) -> bool {
        if settings.toolbar.as_ref().is_some_and(|toolbar| toolbar.locked) {
            return false;
        }
        crate::ui::options::set_look(
            settings,
            if self == Self::Legacy {
                crate::ui::options::Look::Legacy
            } else {
                crate::ui::options::Look::Aurora
            },
        );
        settings.toolbar = Some(self.layout(skin));
        settings.toolbar_selection = Some(super::selection::Selection::builtin(self, skin));
        settings.show_bug_report = true;
        settings.chat_thumbnails = crate::settings::ChatThumbnails::TopRight;
        true
    }
}

/// The existing root menus keep their native handlers and localized labels.
#[cfg(test)]
const ROOT_MENUS: [(&str, &str); 6] = [
    ("common-me", "user-filled"),
    ("ui-bars-communicate", "messages-filled"),
    ("common-world", "world-filled"),
    ("common-build", "box"),
    ("common-contents", "briefcase-filled"),
    ("common-help", "help-circle-filled"),
];

const PHOTO_CAMERA: [&str; 4] = ["snapshot", "view", "photo_roll", "immersive"];
const PHOTO_SETTINGS: [&str; 3] = ["graphics_preferences", "quickprefs", "myenvironments"];
const PERSONAL_LIGHTING: &str = "menu:common-world/common-environment/common-personal-lighting";

fn insert_top_block(settings: &mut ToolbarSettings, mut buttons: Vec<ToolButton>, separator: &str) {
    let at = settings
        .top
        .iter()
        .position(|button| button.command == "bar_navigation")
        .unwrap_or(settings.top.len());
    buttons.push(structure::separator(separator));
    settings.top.splice(at..at, buttons);
}

fn button(id: &str) -> Option<ToolButton> {
    catalogue::find(id).filter(|command| command.ready).map(|command| ToolButton {
        command: command.id.into(),
        icon: command.icon.into(),
        tooltip: String::new(),
    })
}

pub const IMPORTS: [(&str, crate::imports::Choice); 6] = [
    ("import_texture", crate::imports::Choice::Texture),
    ("import_sound", crate::imports::Choice::Sound),
    ("import_animation", crate::imports::Choice::Animation),
    ("import_material", crate::imports::Choice::Material),
    ("import_mesh", crate::imports::Choice::Mesh),
    ("import_local", crate::imports::Choice::Local),
];

pub fn import_choice(command: &str) -> Option<crate::imports::Choice> {
    IMPORTS.iter().find(|(id, _)| *id == command).map(|(_, choice)| *choice)
}

pub fn demo_mode(mode: &str) -> Option<Preset> {
    match mode.trim_end_matches("-edit").trim_end_matches("-advanced") {
        "preset-light" => Some(Preset::Light),
        "preset-old-school" => Some(Preset::OldSchool),
        "preset-build" => Some(Preset::Build),
        "preset-photographer" => Some(Preset::Photographer),
        "preset-legacy" => Some(Preset::Legacy),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        settings::{ChatStyle, ChatThumbnails},
        ui::{
            Panels, bars,
            toolbar::{Bar, catalogue_view},
        },
    };

    #[test]
    fn presets_apply_the_shared_look_and_keep_non_interface_preferences() {
        for preset in Preset::ALL {
            assert!(crate::ui::icons::embedded(preset.icon()));
            let mut settings = Settings {
                draw_distance: 222.0,
                language: "ja".into(),
                ..Default::default()
            };
            assert!(preset.apply(&mut settings, &Layout::default()));
            assert_eq!(settings.classic_bars, preset == Preset::Legacy);
            assert_eq!(settings.classic_inventory, preset == Preset::Legacy);
            assert_eq!(
                settings.chat_style,
                if preset == Preset::Legacy {
                    ChatStyle::Legacy
                } else {
                    ChatStyle::Aurora
                }
            );
            assert_eq!(settings.draw_distance, 222.0);
            assert_eq!(settings.language, "ja");
            assert_eq!(settings.chat_thumbnails, ChatThumbnails::TopRight);
            let toolbar = settings.toolbar.as_ref().expect("preset layout");
            assert_eq!(toolbar.block_styles["bar_chat"].align, Some(Align::Start));
            if preset != Preset::Legacy {
                let editor = toolbar
                    .top
                    .split(|button| structure::is_separator(&button.command))
                    .find(|group| group.iter().any(|button| button.command == "edit_interface"))
                    .expect("Aurora presets expose interface customization");
                // the Aurora layout's block also shows and hides the quick
                // import bar (human request, 2026-11-25)
                let commands: Vec<_> = editor.iter().map(|button| button.command.as_str()).collect();
                if matches!(preset, Preset::Light | Preset::Photographer) {
                    let import = crate::ui::toolbar::drawers::IMPORT_BAR;
                    assert_eq!(commands, ["edit_interface", import]);
                    assert_eq!(toolbar.drawers[import].buttons.len(), 9, "its buttons");
                    assert!(toolbar.shortcuts.contains_key(import), "its Ctrl+U");
                } else {
                    assert_eq!(commands, ["edit_interface"], "the editor has its own block");
                }
            }
            let all = Bar::ALL
                .into_iter()
                .flat_map(|bar| toolbar.bar(bar, &Layout::default()).to_vec())
                .collect::<Vec<_>>();
            let commands = all.iter().map(|button| &button.command).collect::<std::collections::HashSet<_>>();
            assert_eq!(all.len(), commands.len(), "no duplicate commands in {preset:?}");
            let restored: ToolbarSettings = serde_json::from_str(&serde_json::to_string(toolbar).expect("save")).expect("load");
            assert_eq!(&restored, toolbar);
        }
        let mut settings = Settings {
            toolbar: Some(ToolbarSettings {
                locked: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        let before = settings.toolbar.clone();
        assert!(!Preset::Legacy.apply(&mut settings, &Layout::default()));
        assert_eq!(settings.toolbar, before);
        assert!(!settings.classic_bars);
    }

    #[test]
    fn photographer_groups_focus_on_capture_graphics_and_local_lighting() {
        let skin = Layout::default();
        let toolbar = Preset::Photographer.layout(&skin);
        let aurora = Preset::Light.layout(&skin);
        assert_eq!(toolbar.buttons, aurora.buttons, "identical Aurora bottom controls and order");
        assert_eq!(toolbar.bottom_style, aurora.bottom_style);
        for command in ["bar_chat", "bar_conversations"] {
            assert_eq!(toolbar.block_styles.get(command), aurora.block_styles.get(command));
        }
        assert_eq!(
            toolbar.left.iter().map(|button| button.command.as_str()).collect::<Vec<_>>(),
            ["photo_roll", "immersive"],
            "camera and snapshot already live in the shared bottom bar"
        );
        let settings = toolbar
            .top
            .split(|button| structure::is_separator(&button.command))
            .find(|group| group.iter().any(|button| button.command == "graphics_preferences"))
            .expect("photography settings block");
        let mut expected = PHOTO_SETTINGS.to_vec();
        expected.push(PERSONAL_LIGHTING);
        assert_eq!(settings.iter().map(|button| button.command.as_str()).collect::<Vec<_>>(), expected);
        assert!(
            !toolbar
                .buttons(&skin)
                .iter()
                .any(|button| PHOTO_SETTINGS.contains(&button.command.as_str()))
        );
        assert!(!toolbar.top.iter().any(|button| button.command.starts_with("import_")));
        let ctx = egui::Context::default();
        let mut panels = Panels::from_settings(&Settings::default());
        assert!(matches!(
            bars::run_command(&ctx, "graphics_preferences", &mut panels),
            bars::BarAction::GraphicsPreferences
        ));
        bars::run_command(&ctx, "quickprefs", &mut panels);
        assert!(panels.quick_prefs);
        bars::run_command(&ctx, "myenvironments", &mut panels);
        assert!(panels.environment);
        assert!(matches!(
            bars::run_command(&ctx, "photo_roll", &mut panels),
            bars::BarAction::PhotoRoll
        ));
        for command in PHOTO_CAMERA.into_iter().chain(PHOTO_SETTINGS) {
            assert!(catalogue::find(command).is_some_and(|command| command.ready));
        }
    }

    #[test]
    fn build_imports_are_one_block_and_all_are_advanced_native_type_actions() {
        let skin = Layout::default();
        let toolbar = Preset::Build.layout(&skin);
        let imports = toolbar
            .top
            .split(|button| structure::is_separator(&button.command))
            .find(|group| group.iter().any(|button| button.command == "import_texture"))
            .expect("import block");
        assert_eq!(
            imports
                .iter()
                .filter(|button| import_choice(&button.command).is_some())
                .map(|button| button.command.as_str())
                .collect::<Vec<_>>(),
            IMPORTS.iter().map(|(id, _)| *id).collect::<Vec<_>>()
        );
        let tiles = catalogue_view::entries(&skin, &toolbar, &Default::default());
        let ctx = egui::Context::default();
        let mut panels = Panels::from_settings(&Settings::default());
        for (id, expected) in IMPORTS {
            let tile = tiles.iter().find(|tile| tile.button.command == id).expect("advanced import option");
            assert!(tile.in_view(true, "") && !tile.in_view(false, ""));
            assert!(catalogue::find(id).is_some_and(|command| command.ready));
            assert!(matches!(bars::run_command(&ctx, id, &mut panels), bars::BarAction::ImportType(choice) if choice == expected));
        }
        let old = Preset::OldSchool.layout(&skin);
        let legacy = Preset::Legacy.layout(&skin);
        assert_eq!(
            old.buttons(&skin)
                .iter()
                .filter(|button| button.command != "vr_view")
                .map(|button| button.command.as_str())
                .collect::<Vec<_>>(),
            legacy
                .buttons(&skin)
                .iter()
                .map(|button| button.command.as_str())
                .collect::<Vec<_>>(),
            "authored OldSchool retains the traditional command order with explicit VR"
        );
        assert_eq!(
            old.left.iter().map(|button| button.command.as_str()).collect::<Vec<_>>(),
            ["places"]
        );
        assert_eq!(old.top[0].command, "aurora_menu");
        assert!(structure::is_separator(&old.top[1].command));
        assert_eq!(old.top[2].command, "report_bug");
        assert!(structure::is_separator(&old.top[3].command));
        assert_eq!(old.top[4].command, "edit_interface");
        assert!(structure::is_separator(&old.top[5].command));
        let start = old
            .top
            .iter()
            .position(|button| button.command == "menu:common-me")
            .expect("first root menu");
        let menus = old.top[start..]
            .split(|button| structure::is_separator(&button.command))
            .next()
            .expect("text menu block");
        assert_eq!(menus.len(), ROOT_MENUS.len() * 2 - 1);
        for (index, (key, _)) in ROOT_MENUS.iter().enumerate() {
            let button = &menus[index * 2];
            assert_eq!(button.command, format!("menu:{key}"));
            assert_eq!(button.tooltip, format!("@{key}"));
            assert_eq!(old.block_styles[&button.command].mode, Some(ButtonMode::TextOnly));
            assert_eq!(old.block_styles[&button.command].sizing, Some(Sizing::Auto));
            if index > 0 {
                assert!(structure::is_divider(&menus[index * 2 - 1].command));
            }
        }
        assert!(structure::is_separator(&old.top[start + menus.len()].command));
        assert_eq!(old.top.iter().filter(|button| button.command == "report_bug").count(), 1);
        assert_eq!(old.top.iter().filter(|button| button.command == "aurora_menu").count(), 1);
        assert_eq!(old.bottom_style.sizing, Some(Sizing::Fill));
        assert!(
            old.buttons(&skin)
                .windows(3)
                .any(|buttons| buttons.iter().map(|button| button.command.as_str()).eq(["speak", "voice", "move"]))
        );
    }
}
