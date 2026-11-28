//! A `secondlife:///app/...` command clicked in a text (`slapp.rs`): what
//! the Legacy Viewer's command handler does with it, run with Aurora's
//! windows (LLGroupHandler → group profile, LLParcelHandler → place
//! profile, LLObjectIMHandler → the remote object inspector,
//! LLInventoryHandler → inventory with the item selected, LLClassifiedHandler
//! and LLEventHandler → their details in Search, LLWorldMapHandler,
//! LLChatCommandHandler, LLSearchHandler, LLAgentHandler;
//! LLCommandHandlerRegistry::dispatch for the refused ones). And the names
//! the labels of these links wait for.

use super::*;
use crate::slapp::{AgentVerb, AppLink, BlockStep};
use crate::ui::context::CtxAction;
use crate::world::notifications::{Data, Kind};
use crate::world::search::{Key, Tab};
use uuid::Uuid;

/// An `unmute` (resident) or `unblock` (object, with the link's name) link
/// waiting for the user's answer.
#[derive(Debug, Clone)]
pub(super) struct UnblockAsk {
    id: Uuid,
    object: Option<String>,
}

impl App {
    /// Reuse the real command dispatch for offline minimize/reopen captures.
    pub(super) fn prepare_uri_restore_demo(&mut self, action: &CtxAction) {
        if !std::env::var("AURORA_DEMO_APPLINKS").is_ok_and(|mode| mode.ends_with(":restore")) {
            return;
        }
        let window = match action {
            CtxAction::Profile(id) => {
                let prefix = if self.settings.classic_profiles {
                    "profile"
                } else {
                    "profile-card"
                };
                format!("{prefix}-{id}")
            }
            CtxAction::AppLink(link) => match link {
                AppLink::Group(id) => format!("group_profile-{id}"),
                AppLink::Parcel(_) => {
                    self.settings.standalone_place_details = false;
                    "places".into()
                }
                AppLink::ObjectIm(_) => "remote_object".into(),
                AppLink::Inventory { .. } | AppLink::InventoryShow => "inventory".into(),
                AppLink::Experience(_) => "experience_profile".into(),
                AppLink::Classified(_) | AppLink::Event(_) | AppLink::Search { .. } => ui::search::WINDOW.into(),
                AppLink::WorldMap { .. } => "worldmap".into(),
                AppLink::Agent { verb: AgentVerb::Im, .. } => "conversations".into(),
                AppLink::Agent { verb: AgentVerb::Pay, .. } => if self.settings.classic_pay_window { "pay_classic" } else { "pay" }.into(),
                AppLink::Agent {
                    verb: AgentVerb::ReportAbuse,
                    ..
                } => "report_abuse".into(),
                _ => return,
            },
            _ => return,
        };
        self.egui_ctx
            .data_mut(|d| d.insert_temp(egui::Id::unique("uri_restore_demo"), (window, action.clone())));
    }

    pub(super) fn demo_uri_restore(&mut self) {
        if !matches!(self.frame_count, 500 | 620 | 660 | 760) {
            return;
        }
        let Some((window, action)) = self
            .egui_ctx
            .data(|d| d.get_temp::<(String, CtxAction)>(egui::Id::unique("uri_restore_demo")))
        else {
            return;
        };
        match self.frame_count {
            620 => ui::widgets::minimize_floater(&self.egui_ctx, &window),
            500 | 660 => ui::context::request(&self.egui_ctx, action),
            _ => {
                let minimized = ui::widgets::floater_minimized(&self.egui_ctx, &window);
                let height = self.egui_ctx.memory(|m| m.area_rect(egui::Id::unique(&window))).map(|r| r.height());
                log::info!("URI restore demo: {window}, minimized={minimized}, height={height:?}");
            }
        }
    }

    pub(super) fn run_app_link(&mut self, link: AppLink) {
        match link {
            AppLink::Agent { id, verb } => self.run_agent_verb(id, verb),
            AppLink::Group(id) => self.group_profile_ui.open(&mut self.world, id),
            AppLink::Parcel(id) => self.show_place_profile(crate::world::place_details::Source::Parcel(id)),
            AppLink::ObjectIm(o) => {
                ui::widgets::expand_floater(&self.egui_ctx, "remote_object");
                self.remote_object = Some(o);
            }
            AppLink::Inventory { id, .. } => {
                // the Legacy Viewer always shows the item
                // (openInventoryPanelAndSetSelection), on the Received tab
                // for Received Items and what it holds (#1604)
                self.inventory_ui.show_notified(&self.world.inventory, id);
                ui::widgets::open_floater(&self.egui_ctx, "inventory", &mut self.panels.inventory);
            }
            AppLink::InventoryShow => ui::widgets::open_floater(&self.egui_ctx, "inventory", &mut self.panels.inventory),
            AppLink::Preferences(path) => {
                self.options_ui.open_setting(path);
                ui::widgets::open_floater(&self.egui_ctx, ui::options::WINDOW, &mut self.panels.settings);
            }
            AppLink::Experience(id) => self.world.experiences.open_profile(id),
            AppLink::Classified(id) => self.show_search_one(Tab::Classifieds, Key::Id(id)),
            AppLink::Event(id) => self.show_search_one(Tab::Events, Key::Event(id)),
            AppLink::WorldMap { region, pos } => self.on_ctx_action(CtxAction::ShowPlace(region, pos)),
            AppLink::Chat { channel, text } => self.send_local_chat(ui::chat::OutgoingChat {
                message: text,
                channel,
                chat_type: aurora_net::ChatType::Normal,
            }),
            AppLink::Search { category, query } => self.run_search_link(&category, &query),
            AppLink::AreaSearch(filters) => {
                self.area_search_ui.open_filters(*filters);
                ui::widgets::open_floater(&self.egui_ctx, ui::area_search::WINDOW, &mut self.panels.area_search);
            }
            // UNTRUSTED_BLOCK from a text received from someone else; the
            // Legacy Viewer says it once per session, Aurora at each click
            // so that a click never seems lost
            AppLink::Blocked(_) => {
                self.world
                    .notifications
                    .push(Kind::Info, aurora_i18n::tr!("ui-chat-applink-blocked"), "", Data::None);
            }
            AppLink::Unsupported => {
                self.world
                    .notifications
                    .push(Kind::Info, aurora_i18n::tr!("ui-chat-applink-unsupported"), "", Data::None);
            }
        }
    }

    /// LLAgentHandler::handle, the verbs besides the profile. An `unmute` or
    /// `unblock` asks first (`BlockStep::AskUnblock`), whether the link was
    /// clicked in a text or came from outside the viewer.
    fn run_agent_verb(&mut self, id: Uuid, verb: AgentVerb) {
        match verb.block_step(self.world.object_block_entry(&id).is_some()) {
            BlockStep::Nothing => return,
            BlockStep::AskUnblock => {
                let object = match verb {
                    AgentVerb::Unblock(name) => Some(name),
                    _ => None,
                };
                log::info!("app link: asking before unblocking");
                self.unblock_link_ask = Some(UnblockAsk { id, object });
                return;
            }
            BlockStep::Block | BlockStep::Other => {}
        }
        let act = match verb {
            AgentVerb::Im => CtxAction::Im(id),
            AgentVerb::Pay => CtxAction::PayResident(id),
            AgentVerb::OfferTeleport => CtxAction::OfferTeleport(id),
            AgentVerb::RequestFriend => CtxAction::AddFriend(id),
            AgentVerb::RemoveFriend => CtxAction::RemoveFriend(id),
            AgentVerb::Mute | AgentVerb::Unmute => CtxAction::ToggleBlock(id),
            AgentVerb::Block(name) | AgentVerb::Unblock(name) => CtxAction::ToggleBlockObject { id, name },
            AgentVerb::ReportAbuse => CtxAction::ReportAbuse(crate::report_abuse::ReportTarget::Avatar(id)),
        };
        self.on_ctx_action(act);
    }

    /// Each frame: the question of an `unmute` or `unblock` link. On
    /// "Unblock", the block is lifted only if it is still there (it may have
    /// been lifted by hand meanwhile: a toggle would block again).
    pub(super) fn unblock_link_question(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        use crate::ui::widgets::{self, ActionKind};
        let Some(ask) = &self.unblock_link_ask else {
            return;
        };
        let text = match &ask.object {
            None => {
                let name = self
                    .world
                    .legacy_name(&ask.id)
                    .unwrap_or_else(|| self.world.social.name_of(&ask.id));
                aurora_i18n::tr!("ui-chat-applink-unblock-confirm", name = name)
            }
            Some(name) if name.is_empty() => aurora_i18n::tr!(
                "ui-chat-applink-unblock-object-confirm",
                name = aurora_i18n::tr!("common-object-2", value0 = ask.id.to_string()[..8].to_string())
            ),
            Some(name) => aurora_i18n::tr!("ui-chat-applink-unblock-object-confirm", name = name.clone()),
        };
        let mut done = None;
        let modal = widgets::Modal::new(egui::Id::unique(("unblock_link_confirm", ask.id))).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(egui::RichText::new(&text).size(12.5).color(p.ink));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let size = egui::Vec2::new(0.0, 26.0);
                if widgets::action_button(
                    ui,
                    p,
                    "circle-check-filled",
                    aurora_i18n::tr!("common-unblock"),
                    ActionKind::Primary,
                    size,
                )
                .clicked()
                {
                    done = Some(true);
                }
                if widgets::action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                    done = Some(false);
                }
            });
        });
        if modal.should_close() && done.is_none() {
            done = Some(false);
        }
        let Some(yes) = done else {
            return;
        };
        let Some(ask) = self.unblock_link_ask.take() else {
            return;
        };
        if !yes {
            log::info!("app link: unblocking cancelled");
            return;
        }
        if self.world.object_block_entry(&ask.id).is_none() {
            return;
        }
        log::info!("app link: unblocking confirmed");
        self.on_ctx_action(match ask.object {
            None => CtxAction::ToggleBlock(ask.id),
            Some(name) => CtxAction::ToggleBlockObject { id: ask.id, name },
        });
    }

    /// A classified or an event in Search, alone in its tab.
    fn show_search_one(&mut self, tab: Tab, key: Key) {
        self.search_ui.open_tab(tab);
        ui::widgets::open_floater(&self.egui_ctx, ui::search::WINDOW, &mut self.panels.search);
        self.world.search.show_one(tab, key);
        self.search_ui.aurora_show_one(tab, key, &self.settings.search);
    }

    /// LLSearchHandler: Search on the category ("all", "standard",
    /// "destinations" or none: everything) with the query, searched at once.
    fn run_search_link(&mut self, category: &str, query: &str) {
        let tab = match category.trim().to_lowercase().as_str() {
            "classified" => Some(Tab::Classifieds),
            c => Tab::from_name(c).filter(|t| *t != Tab::Web),
        };
        match tab {
            Some(tab) => self.search_ui.open_tab(tab),
            None if self.settings.classic_search => self.search_ui.open_tab(Tab::People),
            None => self.search_ui.open_all(),
        }
        ui::widgets::open_floater(&self.egui_ctx, ui::search::WINDOW, &mut self.panels.search);
        if query.trim().is_empty() {
            return;
        }
        self.search_ui.set_text(tab.unwrap_or(Tab::People), query);
        let requests = if self.settings.classic_search {
            self.search_ui
                .request(tab.unwrap_or(Tab::People), &self.settings.search)
                .into_iter()
                .collect()
        } else {
            self.search_ui.aurora_requests(&self.settings.search)
        };
        let access = self.world.agent_access();
        for request in requests {
            self.world.search.find(request, access, Instant::now());
        }
    }

    /// The names the app links of the texts wait for: avatars, groups
    /// (UUIDGroupNameRequest), experiences, parcels (ParcelInfoRequest,
    /// once each, as LLUrlEntryParcel::sendParcelInfoRequest).
    pub(super) fn ask_app_link_names(&mut self) {
        let wanted = ui::chat::take_app_link_wants();
        for id in wanted.agents {
            self.world.social.want_name(id);
        }
        for id in wanted.groups {
            self.world.land.group_name(&id, &self.world.groups.groups);
        }
        for id in wanted.experiences {
            self.world.land.experience_name(&id);
        }
        for id in wanted.parcels {
            if !self.world.profiles.parcels.contains_key(&id) && self.app_link_parcels.insert(id) {
                self.send(NetCommand::ParcelInfoRequest(id));
            }
        }
    }
}
