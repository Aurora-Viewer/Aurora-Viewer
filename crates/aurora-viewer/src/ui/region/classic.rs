//! The classic "Region / Estate" window, chosen in Preferences › Aurora: the
//! Legacy Viewer's eight tabs in their order (floater_region_info.xml:
//! Region, Debug, Terrain, Estate, Covenant, Experiences, Access,
//! Environment; LLFloaterRegionInfo in newview/llfloaterregioninfo.cpp,
//! originally LGPL 2.1), each showing the settings of its panel with the
//! controls, rules and messages Aurora's window uses: one data model, one
//! set of requests, two layouts.

use super::{RegionUi, View, estate_wide, interventions, manage, overview, people};
use crate::theme::Palette;
use crate::ui::widgets;
use crate::world::World;
use egui::Vec2;
use std::collections::HashMap;
use uuid::Uuid;

/// Id of the classic window (its own place and size).
pub const WINDOW: &str = "region_estate_classic";
pub(super) const SIZE: Vec2 = Vec2::new(820.0, 660.0);
pub(super) const MIN_SIZE: Vec2 = Vec2::new(640.0, 470.0);

/// The Legacy Viewer's tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Tab {
    #[default]
    Region,
    Debug,
    Terrain,
    Estate,
    Covenant,
    Experiences,
    Access,
    Environment,
}

impl Tab {
    pub(super) const ALL: [Tab; 8] = [
        Tab::Region,
        Tab::Debug,
        Tab::Terrain,
        Tab::Estate,
        Tab::Covenant,
        Tab::Experiences,
        Tab::Access,
        Tab::Environment,
    ];

    fn label(self) -> &'static str {
        match self {
            Tab::Region => aurora_i18n::tr!("region-classic-region"),
            Tab::Debug => aurora_i18n::tr!("region-classic-debug"),
            Tab::Terrain => aurora_i18n::tr!("region-classic-terrain"),
            Tab::Estate => aurora_i18n::tr!("region-classic-estate"),
            Tab::Covenant => aurora_i18n::tr!("region-manage-covenant"),
            Tab::Experiences => aurora_i18n::tr!("ui-land-manage-experiences"),
            Tab::Access => aurora_i18n::tr!("region-classic-access"),
            Tab::Environment => aurora_i18n::tr!("common-environment"),
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Tab::Region => "shield-filled",
            Tab::Debug => "code",
            Tab::Terrain => "mountain-filled",
            Tab::Estate => "door-enter",
            Tab::Covenant => "gavel",
            Tab::Experiences => "flask-filled",
            Tab::Access => "users",
            Tab::Environment => "sun-high-filled",
        }
    }

    /// The tab of a Manage section of the demo (`demo_region.rs`'s
    /// numbering) or of People.
    pub fn of_section(section: Option<u8>, people: bool) -> Tab {
        if people {
            return Tab::Access;
        }
        match section {
            Some(0) => Tab::Estate,
            Some(2) => Tab::Terrain,
            Some(3) | Some(4) => Tab::Debug,
            Some(5) => Tab::Environment,
            Some(6) => Tab::Experiences,
            Some(7) => Tab::Covenant,
            _ => Tab::Region,
        }
    }
}

/// The window's body: the tabs on the left, the panel on the right under
/// the region's name.
#[allow(clippy::too_many_arguments)]
pub(super) fn body(
    ui: &mut egui::Ui,
    p: &Palette,
    s: &mut RegionUi,
    v: &View,
    world: &mut World,
    pics: &overview::Pictures<'_>,
    avatars: &HashMap<Uuid, egui::TextureHandle>,
    images: &HashMap<Uuid, egui::TextureHandle>,
) {
    let height = ui.available_height();
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(150.0, height), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(150.0);
            for tab in Tab::ALL {
                if widgets::navigation(ui, p, tab.icon(), tab.label(), s.classic_tab == tab).clicked() {
                    s.classic_tab = tab;
                }
            }
        });
        widgets::divider(ui, p);
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical()
                .id_salt(("region_estate_classic", s.classic_tab))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_width(ui.available_width() - 6.0);
                    // the panels' "Region:" line: the header keeps the name
                    overview::header(ui, p, s, v, world, pics, true, true);
                    ui.add_space(8.0);
                    manage::demo(s, v, world);
                    panel(ui, p, s, v, world, avatars, images);
                    ui.add_space(4.0);
                });
        });
    });
}

#[allow(clippy::too_many_arguments)]
fn panel(
    ui: &mut egui::Ui,
    p: &Palette,
    s: &mut RegionUi,
    v: &View,
    world: &mut World,
    avatars: &HashMap<Uuid, egui::TextureHandle>,
    images: &HashMap<Uuid, egui::TextureHandle>,
) {
    match s.classic_tab {
        // LLPanelRegionGeneralInfo: the rules, the telehub; its teleports
        // home and message are with the other interventions on Debug
        Tab::Region => manage::rules(ui, p, s, v, world),
        // LLPanelRegionDebugInfo: the switches, Top lists, return, restart
        Tab::Debug => {
            manage::scripts(ui, p, s, v, world);
            ui.add_space(12.0);
            interventions::section(ui, p, s, v, world);
        }
        Tab::Terrain => manage::terrain(ui, p, s, v, world, images),
        Tab::Estate => manage::access(ui, p, s, v, world),
        Tab::Covenant => estate_wide::covenant(ui, p, s, v, world),
        Tab::Experiences => estate_wide::experiences(ui, p, s, v, world),
        Tab::Access => people::show(ui, p, s, v, world, avatars, images, true),
        Tab::Environment => estate_wide::environment(ui, p, s, v, world),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_demo_sections_open_their_tab() {
        assert_eq!(Tab::of_section(Some(0), false), Tab::Estate);
        assert_eq!(Tab::of_section(Some(4), false), Tab::Debug);
        assert_eq!(Tab::of_section(None, true), Tab::Access);
        assert_eq!(Tab::of_section(None, false), Tab::Region);
        assert_eq!(Tab::ALL.len(), 8);
    }
}
