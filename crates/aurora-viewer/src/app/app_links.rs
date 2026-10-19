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
use crate::slapp::{AgentVerb, AppLink};
use crate::ui::context::CtxAction;
use crate::world::notifications::{Data, Kind};
use crate::world::search::{Key, Tab};
use uuid::Uuid;

impl App {
    pub(super) fn run_app_link(&mut self, link: AppLink) {
        match link {
            AppLink::Agent { id, verb } => self.run_agent_verb(id, verb),
            AppLink::Group(id) => self.group_profile_ui.open(&mut self.world, id),
            AppLink::Parcel(id) => self.show_place_profile(crate::world::place_details::Source::Parcel(id)),
            AppLink::ObjectIm(o) => self.remote_object = Some(o),
            AppLink::Inventory { id, .. } => {
                // the Legacy Viewer always shows the item
                // (openInventoryPanelAndSetSelection)
                self.inventory_ui.show_original(&self.world.inventory, id);
                self.panels.inventory = true;
            }
            AppLink::InventoryShow => self.panels.inventory = true,
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

    /// LLAgentHandler::handle, the verbs besides the profile.
    fn run_agent_verb(&mut self, id: Uuid, verb: AgentVerb) {
        let blocked = self.world.mutes.is_muted_id(&id);
        let act = match verb {
            AgentVerb::Im => CtxAction::Im(id),
            AgentVerb::Pay => CtxAction::PayResident(id),
            AgentVerb::OfferTeleport => CtxAction::OfferTeleport(id),
            AgentVerb::RequestFriend => CtxAction::AddFriend(id),
            AgentVerb::RemoveFriend => CtxAction::RemoveFriend(id),
            AgentVerb::Mute | AgentVerb::Unmute if blocked == matches!(verb, AgentVerb::Mute) => return,
            AgentVerb::Mute | AgentVerb::Unmute => CtxAction::ToggleBlock(id),
            AgentVerb::Block(_) | AgentVerb::Unblock(_) if blocked == matches!(verb, AgentVerb::Block(_)) => return,
            AgentVerb::Block(name) | AgentVerb::Unblock(name) => CtxAction::ToggleBlockObject { id, name },
            AgentVerb::ReportAbuse => CtxAction::ReportAbuse(crate::report_abuse::ReportTarget::Avatar(id)),
        };
        self.on_ctx_action(act);
    }

    /// A classified or an event in Search, alone in its tab.
    fn show_search_one(&mut self, tab: Tab, key: Key) {
        self.search_ui.open_tab(tab);
        self.panels.search = true;
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
        self.panels.search = true;
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

    /// The names the app links of the texts wait for: groups
    /// (UUIDGroupNameRequest), experiences, parcels (ParcelInfoRequest,
    /// once each, as LLUrlEntryParcel::sendParcelInfoRequest).
    pub(super) fn ask_app_link_names(&mut self) {
        let wanted = ui::chat::take_app_link_wants();
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
