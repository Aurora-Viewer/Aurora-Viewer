//! User-requested layouts over the shared toolbar and interface-look settings.
//! The traditional command order follows skins/default/toolbars.xml and the
//! native upload types follow indra/newview/llviewermenufile.cpp.

use std::sync::LazyLock;

use super::ToolbarSettings;
use crate::{settings::Settings, ui::skin::Layout};

// The built-in interfaces are the human's saved layouts (2026-11-26), in the
// interface notecard format, validated by the preset and profile tests. Keep
// captions live through the catalogue, rather than freezing an export's
// locale. The Aurora Builder interface is gone with them.
static AURORA: LazyLock<ToolbarSettings> = LazyLock::new(|| bundled(include_str!("../../../assets/toolbar/aurora.json")));
static OLD_SCHOOL: LazyLock<ToolbarSettings> = LazyLock::new(|| bundled(include_str!("../../../assets/toolbar/aurora-old-school.json")));
static PHOTOGRAPHER: LazyLock<ToolbarSettings> =
    LazyLock::new(|| bundled(include_str!("../../../assets/toolbar/aurora-photographer.json")));
static LEGACY: LazyLock<ToolbarSettings> = LazyLock::new(|| bundled(include_str!("../../../assets/toolbar/legacy.json")));

fn bundled(text: &str) -> ToolbarSettings {
    let mut toolbar = super::profile::read(text)
        .expect("bundled toolbar preset must be a valid interface profile")
        .toolbar;
    toolbar.migrate_interfaces();
    toolbar
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    Light,
    OldSchool,
    Photographer,
    Legacy,
}

impl Preset {
    pub const ALL: [Self; 4] = [Self::Light, Self::OldSchool, Self::Photographer, Self::Legacy];

    pub fn label(self) -> &'static str {
        aurora_i18n::text(match self {
            Self::Light => "toolbar-preset-light",
            Self::OldSchool => "toolbar-preset-old-school",
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
            Self::Photographer => "camera-filled",
            Self::Legacy => "layout-grid-filled",
        }
    }

    /// The same for every skin: the bundled layouts name their bars' buttons.
    pub fn layout(self, _skin: &Layout) -> ToolbarSettings {
        match self {
            Self::Light => AURORA.clone(),
            Self::OldSchool => OLD_SCHOOL.clone(),
            Self::Photographer => PHOTOGRAPHER.clone(),
            Self::Legacy => LEGACY.clone(),
        }
    }

    /// The interface lock never stops a change of interface (human request,
    /// 2026-11-26): the new one keeps it (`keep_interface_choices`).
    pub fn apply(self, settings: &mut Settings, skin: &Layout) -> bool {
        crate::ui::options::set_look(
            settings,
            if self == Self::Legacy {
                crate::ui::options::Look::Legacy
            } else {
                crate::ui::options::Look::Aurora
            },
        );
        let mut toolbar = self.layout(skin);
        toolbar.keep_interface_choices(settings.toolbar.as_ref());
        settings.toolbar = Some(toolbar);
        settings.toolbar_selection = Some(super::selection::Selection::builtin(self, skin));
        settings.show_bug_report = true;
        settings.chat_thumbnails = crate::settings::ChatThumbnails::TopRight;
        true
    }
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
            toolbar::{Bar, catalogue, catalogue_view, drawers, profile},
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
            // every interface reaches the others through its Interfaces block
            assert!(toolbar.drawers.contains_key(drawers::INTERFACES), "{preset:?}");
            assert!(toolbar.placed(drawers::INTERFACES, &Layout::default()), "{preset:?}");
            if preset == Preset::Legacy {
                let top: Vec<_> = toolbar.top.iter().map(|button| button.command.as_str()).collect();
                assert_eq!(
                    top[..3],
                    ["aurora_menu", "report_bug", drawers::INTERFACES],
                    "the Interfaces block right of Report a bug"
                );
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
        // the lock never stops a change of interface, which keeps it (human
        // request, 2026-11-26)
        assert!(Preset::Legacy.apply(&mut settings, &Layout::default()));
        assert!(settings.classic_bars);
        let toolbar = settings.toolbar.as_ref().expect("the Legacy layout");
        assert!(toolbar.locked, "still locked");
        assert_eq!(toolbar.left, Preset::Legacy.layout(&Layout::default()).left);
    }

    /// The built-in interfaces are the human's saved layouts (2026-11-26),
    /// each on its own base, the personal interfaces of whoever saved them
    /// left out; the Aurora Builder interface is gone.
    #[test]
    fn the_built_in_interfaces_are_the_bundled_saved_layouts() {
        let files = [
            (Preset::Light, include_str!("../../../assets/toolbar/aurora.json")),
            (Preset::OldSchool, include_str!("../../../assets/toolbar/aurora-old-school.json")),
            (
                Preset::Photographer,
                include_str!("../../../assets/toolbar/aurora-photographer.json"),
            ),
            (Preset::Legacy, include_str!("../../../assets/toolbar/legacy.json")),
        ];
        assert_eq!(files.map(|(preset, _)| preset), Preset::ALL);
        for (preset, text) in files {
            let saved = profile::read(text).expect("a valid interface");
            assert_eq!(saved.base, profile::Base::from_islands(preset != Preset::Legacy), "{preset:?}");
            assert!(saved.toolbar.hidden_interfaces.is_empty(), "{preset:?}");
            assert!(!saved.toolbar.locked, "{preset:?}");
            assert_eq!(preset.layout(&Layout::default()), bundled(text), "{preset:?}");
        }
        assert!(Preset::from_key("build").is_none());
    }

    #[test]
    fn imports_are_advanced_native_type_actions() {
        let skin = Layout::default();
        let tiles = catalogue_view::entries(&skin, &Preset::Light.layout(&skin), &Default::default());
        let ctx = egui::Context::default();
        let mut panels = Panels::from_settings(&Settings::default());
        for (id, expected) in IMPORTS {
            let tile = tiles.iter().find(|tile| tile.button.command == id).expect("advanced import option");
            assert!(tile.in_view(true, "") && !tile.in_view(false, ""));
            assert!(catalogue::find(id).is_some_and(|command| command.ready));
            assert!(matches!(bars::run_command(&ctx, id, &mut panels), bars::BarAction::ImportType(choice) if choice == expected));
        }
    }
}
