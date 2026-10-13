//! Group profile window, first version: the general information that
//! GroupProfileReply carries (LLPanelGroupGeneral: insignia, name, key,
//! founder, charter, our title, enrollment, fee, rating), drawn by the group
//! pane of « Rechercher » (`search/details.rs`) in a window of its own. A
//! group's name opens it like LLGroupActions::show (the "group ... inspect"
//! links of Firestorm): « À propos du terrain », the build window, the
//! search results. One window, on the last group asked for. The members,
//! roles, notices and land panels of LLPanelGroup are not ported yet
//! (indra/newview/llpanelgroup.cpp, llgroupactions.cpp, originally LGPL 2.1).

use super::place_details::Ctx;
use super::search::SearchAction;
use super::widgets::Floater;
use crate::world::World;
use egui::Vec2;
use uuid::Uuid;

const WINDOW: &str = "group_profile";

/// Ask for the profile of a group from anywhere in the interface (a link, a
/// name); the app opens it at the end of the frame.
pub fn request_open(ctx: &egui::Context, id: Uuid) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("aurora_open_group_profile"), id));
}

pub fn take_open_request(ctx: &egui::Context) -> Option<Uuid> {
    ctx.data_mut(|d| d.remove_temp::<Uuid>(egui::Id::new("aurora_open_group_profile")))
}

#[derive(Default)]
pub struct GroupProfileUi {
    group: Option<Uuid>,
    open: bool,
    raise: bool,
}

impl GroupProfileUi {
    /// LLGroupActions::show: the window on this group, brought to the front,
    /// its profile asked again.
    pub fn open(&mut self, world: &mut World, id: Uuid) {
        if id.is_nil() {
            return;
        }
        world.search.request_group(id);
        self.group = Some(id);
        self.open = true;
        self.raise = true;
    }

    /// The actions of the pane's buttons, for `apply_search_actions`.
    pub fn show(&mut self, ctx: &egui::Context, c: &mut Ctx) -> Vec<SearchAction> {
        let mut actions = Vec::new();
        let Some(id) = self.group.filter(|_| self.open) else {
            return actions;
        };
        if std::mem::take(&mut self.raise) {
            ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::new(WINDOW)));
        }
        // until the profile answers: the name of one of our groups, or the
        // one « À propos du terrain » looked up
        let known = c
            .world
            .groups
            .groups
            .iter()
            .find(|g| g.id == id)
            .map(|g| g.name.clone())
            .or_else(|| c.world.land.group_names.get(&id).cloned())
            .unwrap_or_default();
        let size = Vec2::new(430.0, 480.0);
        let screen = ctx.content_rect();
        let mut floater = Floater::new(
            WINDOW,
            aurora_i18n::tr!("ui-group-profile-title"),
            screen.center() - size * 0.5 + Vec2::new(60.0, 20.0),
            size,
        );
        floater.min_size = Vec2::new(320.0, 300.0);
        floater.show(ctx, c.p, &mut self.open, |ui| {
            // the pane fills the height it is given (details over buttons):
            // it gets the window's body, not a layout it could push taller
            let body = egui::Rect::from_min_size(ui.cursor().min, ui.available_size());
            ui.allocate_rect(body, egui::Sense::hover());
            let mut pane = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(body)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            pane.set_clip_rect(body.intersect(ui.clip_rect()));
            super::search::group_details(&mut pane, c, id, &known, true, &mut actions);
        });
        actions
    }
}
