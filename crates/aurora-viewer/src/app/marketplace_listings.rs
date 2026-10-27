//! Application bridge for the Marketplace listings window: the SLM requests
//! on the session (never on this thread), their answers, the alerts in the
//! notifications, and the demo's simulated SLM server.

use super::*;
use crate::world::marketplace::{self as mp, demo};
use crate::world::notifications::{Data, Kind};
use ui::inventory::InvAction;
use ui::marketplace_listings::Action;

impl App {
    pub(super) fn prepare_inventory_marketplace_commands(&mut self) {
        self.inventory_ui.marketplace_commands.clear();
        let Some(root) = mp::root(&self.world.inventory) else { return };
        let ids: std::collections::HashSet<_> = self
            .inventory_ui
            .selected
            .into_iter()
            .chain(self.inventory_ui.windows.iter().filter_map(|window| window.state.selected))
            .collect();
        if ids.iter().any(|id| mp::depth(&self.world.inventory, root, *id) >= 0) {
            if self.listings_ui.data.initialize() {
                self.send(NetCommand::RequestMerchantStatus);
            }
            if self.listings_ui.data.is_merchant() {
                self.listings_ui.data.fetch_contents(root);
            }
        }
        for id in ids {
            let options = self.listings_ui.inventory_commands(&self.world.inventory, root, id);
            if !options.is_empty() {
                self.inventory_ui.marketplace_commands.insert(id, options);
            }
        }
    }

    pub(super) fn inventory_marketplace_command(&mut self, item: uuid::Uuid, command: ui::marketplace_listings::Command) {
        let Some(root) = mp::root(&self.world.inventory) else { return };
        let worn = self.worn_items();
        let mut actions = Vec::new();
        self.listings_ui.open = true;
        self.listings_ui
            .inventory_command(&self.world.inventory, &worn, root, item, command, &mut actions);
        for action in actions {
            match action {
                Action::Edit(change) => self.apply_inventory_action(InvAction::Edit(change)),
                Action::Paste { change, cut } => {
                    self.inventory_ui.consume_clipboard = cut;
                    self.submit_inventory_change(change);
                }
                Action::OpenMarketplace(url) => self.marketplace_ui.open(Some(url)),
                Action::OpenExternal(url) => self.egui_ctx.open_url(egui::OpenUrl::new_tab(url)),
                Action::Notify { text, url } => self.marketplace_alert(text, url),
                Action::CreateRoot => self.open_marketplace_listings(),
            }
        }
    }
    /// Me › Marketplace listings (LLFloaterMarketplaceListings::onOpen).
    pub(super) fn open_marketplace_listings(&mut self) {
        if self.listings_ui.open() {
            self.send(NetCommand::RequestMerchantStatus);
        }
    }

    /// Every frame: the listings follow the inventory, the queued SLM
    /// requests go to the session, the alerts to the notifications.
    pub(super) fn update_marketplace_listings(&mut self) {
        self.listings_ui.reconcile(&self.world.inventory);
        for request in self.listings_ui.data.take_requests(std::time::Instant::now()) {
            log::info!("SLM request queued: {}", request.name());
            self.send(NetCommand::Slm(request));
        }
        for alert in self.listings_ui.data.take_alerts() {
            let (text, url) = ui::marketplace_listings::alert_text(&alert);
            self.marketplace_alert(text, url);
        }
        if self.demo {
            self.demo_marketplace_seed();
        }
        // Use the application's frame clock: egui's count starts later
        // than the demo and its capture/loading clock.
        if self.demo && self.frame_count >= 680 {
            let mode = std::env::var("AURORA_DEMO_MARKETPLACE_LISTINGS").unwrap_or_default();
            let menu = match mode.as_str() {
                "menu-listing" => Some(demo::HOODIE),
                "menu-version" => Some(demo::HOODIE_V2),
                "menu-item" => Some(uuid::Uuid::from_u128(8_400_102)),
                _ => None,
            };
            if let Some(id) = menu {
                self.listings_ui.demo_menu(id);
            }
        }
        if self.demo && std::env::var("AURORA_DEMO_MARKETPLACE_LISTINGS").is_ok_and(|v| v == "audit") {
            let worn = self.worn_items();
            self.listings_ui.demo_audit(&self.world.inventory, &worn);
        }
    }

    /// NetEvent::MerchantStatus and NetEvent::Slm; false for other events.
    pub(super) fn marketplace_listings_event(&mut self, ev: &NetEvent) -> bool {
        match ev {
            NetEvent::MerchantStatus(status) => {
                log::info!("merchant status: {status:?}");
                self.inventory_ui.merchant = *status == aurora_net::slm::MerchantStatus::Merchant;
                self.listings_ui.data.set_status(status.clone());
                true
            }
            NetEvent::Slm { request, reply } => {
                let root = mp::root(&self.world.inventory).unwrap_or_default();
                self.listings_ui
                    .data
                    .apply(&self.world.inventory, root, request, reply.clone(), std::time::Instant::now());
                true
            }
            _ => false,
        }
    }

    fn worn_items(&self) -> std::collections::HashSet<uuid::Uuid> {
        ui::appearance::items::worn_labels(&self.world).into_keys().collect()
    }

    fn marketplace_alert(&mut self, text: String, url: Option<String>) {
        self.world.notifications.push(
            Kind::Alert,
            aurora_i18n::tr!("marketplace-listings-title"),
            text,
            url.map_or(Data::None, Data::Url),
        );
    }

    pub(super) fn show_marketplace_listings(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if !self.listings_ui.open {
            return;
        }
        let worn = self.worn_items();
        let home = self.marketplace_home();
        let preferences = self.settings.inventory.clone();
        let mut window = self.inventory_ui.take_listings_panel();
        let facts = ui::inventory::Facts::from_world(&self.world, self.appearance_ui.pending.is_some());
        let mut panel = ui::inventory::listings::Panel {
            state: &mut window.state,
            prefs: &mut self.settings.inventory,
            facts: &facts,
            actions: Vec::new(),
        };
        let actions = self.listings_ui.show(
            ctx,
            p,
            &self.skin.icons,
            &mut self.world.inventory,
            &worn,
            home.as_deref(),
            &mut panel,
        );
        let inventory_actions = std::mem::take(&mut panel.actions);
        self.inventory_ui.put_listings_panel(window);
        self.inventory_ui.preferences_dirty |= self.settings.inventory != preferences;
        for action in inventory_actions {
            self.apply_inventory_action(action);
        }
        for action in actions {
            match action {
                Action::CreateRoot => {
                    let id = uuid::Uuid::new_v4();
                    let root = self.world.inventory.root;
                    self.apply_inventory_action(InvAction::Edit(aurora_net::inventory::operations::Mutation {
                        operations: vec![aurora_net::inventory::operations::Operation::Folder {
                            id,
                            parent: root,
                            name: aurora_i18n::tr!("common-marketplace-listings").into(),
                            type_default: mp::LISTINGS,
                        }],
                        refresh: vec![root, id],
                        ..Default::default()
                    }));
                }
                Action::Edit(change) => self.submit_inventory_change(change),
                Action::Paste { change, cut } => {
                    self.inventory_ui.consume_clipboard = cut;
                    self.submit_inventory_change(change);
                }
                Action::OpenMarketplace(url) => self.marketplace_ui.open(Some(url)),
                Action::OpenExternal(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
                Action::Notify { text, url } => self.marketplace_alert(text, url),
            }
        }
        if !self.inventory_ui.message.is_empty() && self.listings_ui.message.is_empty() {
            self.listings_ui.message.clone_from(&self.inventory_ui.message);
        }
    }

    pub(super) fn paste_marketplace_inventory(&mut self, to: uuid::Uuid, ids: Vec<uuid::Uuid>, copy: bool) {
        let worn = self.worn_items();
        let mut actions = Vec::new();
        self.listings_ui.paste_inventory(
            &self.world.inventory,
            &worn,
            &self.settings.inventory.protected,
            to,
            &ids,
            copy,
            false,
            &mut actions,
        );
        for action in actions {
            match action {
                Action::Paste { change, cut } => {
                    self.inventory_ui.consume_clipboard = cut;
                    self.submit_inventory_change(change);
                }
                Action::Notify { text, url } => self.marketplace_alert(text, url),
                _ => {}
            }
        }
    }

    /// AURORA_DEMO_MARKETPLACE_LISTINGS (docs/TESTING.md): the sample
    /// listings and the window open at start.
    pub(super) fn demo_marketplace_listings(&mut self) {
        let Some(mode) = demo::Mode::from_env() else {
            return;
        };
        self.demo_slm = Some(demo::DemoSlm::new(mode));
        // a merchant's window opens once the sample listings are in the
        // inventory (`demo_marketplace_seed`)
        if mode != demo::Mode::Merchant {
            self.open_marketplace_listings();
        }
    }

    /// The sample listings, once the demo inventory has its root.
    fn demo_marketplace_seed(&mut self) {
        let Some(slm) = self.demo_slm.as_mut() else { return };
        if slm.mode != demo::Mode::Merchant || slm.seeded || self.world.inventory.root.is_nil() {
            return;
        }
        slm.seeded = true;
        demo::seed(&mut self.world.inventory, self.world.agent_id);
        self.listings_ui
            .demo_expand(&[demo::HOODIE, demo::HOODIE_V2, demo::POSTER, demo::POSTER_VERSION, demo::LANTERN]);
        self.open_marketplace_listings();
    }

    /// The demo's SLM server: GET /merchant and the listings requests are
    /// answered in the viewer, from the sample listings.
    pub(super) fn demo_marketplace_command(&mut self, cmd: &NetCommand) -> bool {
        let slm = self
            .demo_slm
            .get_or_insert_with(|| demo::DemoSlm::new(demo::Mode::from_env().unwrap_or(demo::Mode::NotMerchant)));
        match cmd {
            NetCommand::RequestMerchantStatus => {
                let status = slm.merchant();
                self.on_app_event(NetEvent::MerchantStatus(status));
                true
            }
            NetCommand::Slm(request) => {
                log::info!("demo action: SLM {} (simulated)", request.name());
                let reply = slm.answer(request);
                self.on_app_event(NetEvent::Slm {
                    request: request.clone(),
                    reply,
                });
                true
            }
            _ => false,
        }
    }
}
