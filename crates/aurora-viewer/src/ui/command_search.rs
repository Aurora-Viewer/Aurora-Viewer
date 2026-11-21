use super::bars::{self, BarAction, StatusInfo};
use super::options::Page;
use super::{Panels, menu};
use crate::theme::Palette;
use crate::world::landmarks::SavedPlace;
use aurora_i18n::tr;
use uuid::Uuid;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Preferences(Page, usize),
    Teleport { asset: Uuid, name: String },
    ShowOnMap(Uuid),
    Environment(super::environment::EnvAction),
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Sources<'a> {
    pub places: &'a [SavedPlace],
    pub can_teleport: bool,
    pub can_show_map: bool,
    pub can_show_inventory: bool,
    pub has_photos: bool,
    pub environments: Option<&'a crate::world::env_select::SettingsCatalog>,
}

/// Toolbar actions whose viewer action is missing from the wolf menu.
const TOOLBAR_ONLY: &[&str] = &[
    "inventory_dual",
    "texture_library",
    "group_titles",
    "webbrowser",
    "quickprefs",
    "snapshot_to_disk",
    "photo_roll",
    "360capture",
    "water_walk",
];

#[derive(Default)]
pub(crate) struct CommandSearch {
    open: bool,
    previous_focus: Option<egui::Id>,
}

impl CommandSearch {
    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    pub(crate) fn toggle(&mut self, ctx: &egui::Context) {
        if self.open {
            self.dismiss(ctx);
        } else {
            self.previous_focus = ctx.memory(|m| m.focused());
            self.open = true;
        }
    }

    pub(crate) fn close(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn dismiss(&mut self, ctx: &egui::Context) {
        if let Some(id) = self.previous_focus.take() {
            ctx.memory_mut(|m| m.request_focus(id));
        }
        self.close();
    }

    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        panels: &mut Panels,
        status: &StatusInfo<'_>,
        sources: Sources<'_>,
    ) -> (BarAction, Option<Action>) {
        let mut bar = BarAction::None;
        let mut action = None;
        let screen = ctx.content_rect();
        let width = 360.0_f32.min(screen.width());
        let pos = egui::pos2(screen.center().x - width / 2.0, screen.top() + 72.0);
        menu::popup_at(ctx, egui::Id::unique("command_search"), pos, &mut self.open, p, |ui| {
            ui.set_min_width(width);
            menu::searchable_with_prompt(ui, p, tr!("command-search-title"), true, |ui| {
                bars::main_menus(ui, p, panels, status, &mut bar, true);
                preferences(ui, p, &mut action);
                toolbar(ui, p, panels, &sources, &mut bar);
                if let Some(catalog) = sources.environments {
                    environments(ui, p, catalog, &mut action);
                }
                if !status.location_hidden {
                    saved_places(ui, p, &sources, &mut action);
                }
            });
        });
        if !self.open {
            self.previous_focus = None;
        }
        (bar, action)
    }

    pub(crate) fn demo_input(&mut self, frame: u64, input: &mut egui::RawInput) {
        let Some(query) = demo_query().filter(|query| !matches!(*query, "closed" | "off")) else {
            return;
        };
        let (query, run) = [",run", ",confirm"]
            .into_iter()
            .find_map(|suffix| query.strip_suffix(suffix))
            .map_or((query, false), |q| (q, true));
        match frame {
            620 => self.open = true,
            630 => input.events.push(egui::Event::Text(query.to_owned())),
            650 if run => input.events.push(egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }),
            _ => {}
        }
    }
}

pub(crate) fn demo_query() -> Option<&'static str> {
    static QUERY: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    QUERY.get_or_init(|| std::env::var("AURORA_DEMO_COMMAND_SEARCH").ok()).as_deref()
}

pub(crate) fn demo_confirm() -> bool {
    demo_query().is_some_and(|query| query.ends_with(",confirm"))
}

fn preferences(ui: &mut egui::Ui, p: &Palette, action: &mut Option<Action>) {
    menu::submenu(ui, p, "settings-filled", tr!("common-preferences"), true, |ui| {
        for page in Page::ALL {
            let tabs = page.tabs();
            if (tabs.len() <= 1 || menu::filtering()) && menu::item(ui, p, page.icon(), page.name()) {
                *action = Some(Action::Preferences(page, 0));
            }
            if tabs.len() > 1 {
                menu::submenu(ui, p, page.icon(), page.name(), true, |ui| {
                    for (index, tab) in tabs.iter().enumerate() {
                        if menu::item(ui, p, "", tab) {
                            *action = Some(Action::Preferences(page, index));
                        }
                    }
                });
            }
        }
    });
}

fn environments(ui: &mut egui::Ui, p: &Palette, catalog: &crate::world::env_select::SettingsCatalog, action: &mut Option<Action>) {
    use crate::world::env_select::SettingsKind;
    for (kind, label, icon) in [
        (SettingsKind::Sky, tr!("ui-environment-sky-environment"), "sun-filled"),
        (SettingsKind::Water, tr!("ui-environment-water-environment"), "droplet-filled"),
        (SettingsKind::Day, tr!("extra-day-night"), "sunset-2-filled"),
    ] {
        menu::submenu(ui, p, icon, label, true, |ui| {
            for choice in catalog.list(kind) {
                if menu::item(ui, p, icon, &choice.name) {
                    *action = Some(Action::Environment(super::environment::EnvAction::Pick(kind, choice.asset)));
                }
            }
        });
    }
}

fn toolbar(ui: &mut egui::Ui, p: &Palette, panels: &mut Panels, sources: &Sources<'_>, bar: &mut BarAction) {
    menu::submenu(ui, p, "layout-filled", tr!("toolbar-title"), true, |ui| {
        for command in super::toolbar::COMMANDS.iter().filter(|c| c.ready && TOOLBAR_ONLY.contains(&c.id)) {
            let enabled = match command.id {
                "inventory_dual" => sources.can_show_inventory,
                "photo_roll" => sources.has_photos,
                _ => true,
            };
            if menu::item_if(ui, p, command.icon, command.label(), enabled) {
                *bar = bars::run_command(ui.ctx(), command.id, panels);
            }
        }
    });
}

fn saved_places(ui: &mut egui::Ui, p: &Palette, sources: &Sources<'_>, action: &mut Option<Action>) {
    for (favorite, label, icon) in [
        (true, tr!("common-favorites"), "star-filled"),
        (false, tr!("common-landmarks"), "map-pin-filled"),
    ] {
        if !sources.places.iter().any(|place| place.favorite == favorite) {
            continue;
        }
        menu::submenu(ui, p, icon, label, true, |ui| {
            let matching = || sources.places.iter().filter(|place| place.favorite == favorite);
            for place in matching() {
                menu::search_path(&place.folders, || {
                    if menu::item_detail_if(ui, p, icon, &place.name, &place.folders, sources.can_teleport) {
                        *action = Some(Action::Teleport {
                            asset: place.asset,
                            name: place.name.clone(),
                        });
                    }
                });
            }
            if !sources.can_show_map {
                return;
            }
            menu::submenu(ui, p, "map", tr!("common-show-on-map"), true, |ui| {
                for place in matching() {
                    menu::search_path(&place.folders, || {
                        if menu::item_detail(ui, p, "map", &place.name, &place.folders) {
                            *action = Some(Action::ShowOnMap(place.asset));
                        }
                    });
                }
            });
        });
    }
}
